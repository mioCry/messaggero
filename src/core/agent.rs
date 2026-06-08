use async_trait::async_trait;

use super::error::AgentError;
use super::types::{AgentCard, TaskRequest, TaskResponse, TaskState, TaskStatus};

/// Core trait that every messaggero agent must implement.
///
/// An agent receives task requests and produces task responses, following the
/// A2A lifecycle model (`Submitted` → `Working` → `Completed`/`Failed`/`Canceled`).
///
/// Implement this trait on any `Send + Sync` type. The two required methods are
/// [`card`](Agent::card) and [`handle_task`](Agent::handle_task).
/// [`handle_cancel`](Agent::handle_cancel) has a sensible default.
///
/// # Example
///
/// ```rust,ignore
/// use messaggero::prelude::*;
///
/// struct UpperCaseAgent;
///
/// #[async_trait]
/// impl Agent for UpperCaseAgent {
///     fn card(&self) -> AgentCard {
///         AgentCard::builder("uppercase")
///             .description("Converts text to uppercase")
///             .build()
///     }
///
///     async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
///         let text = req.message.text_content().unwrap_or("").to_uppercase();
///         Ok(TaskResponse::completed(&req.id, Message::agent(text)))
///     }
/// }
/// ```
#[async_trait]
pub trait Agent: Send + Sync {
    /// Returns the agent's capability card (A2A-compatible [`AgentCard`]).
    ///
    /// Called once during server startup to register the agent with the
    /// `Discovery` registry and to populate `GET /.well-known/agent.json`.
    fn card(&self) -> AgentCard;

    /// Handles an incoming task request and returns a response.
    ///
    /// # Errors
    ///
    /// Return [`AgentError`] for any unrecoverable failure. The transport layer
    /// will convert the error into the appropriate wire-level error response.
    async fn handle_task(&self, request: TaskRequest) -> Result<TaskResponse, AgentError>;

    /// Handles a task cancellation request.
    ///
    /// The default implementation immediately returns a `Canceled` status.
    /// Override this method if the agent needs to perform cleanup (e.g. stopping
    /// a background job) before acknowledging the cancellation.
    ///
    /// # Errors
    ///
    /// Return [`AgentError`] if cleanup fails. The cancellation is still
    /// considered acknowledged even if this returns an error.
    async fn handle_cancel(&self, _task_id: &str) -> Result<TaskStatus, AgentError> {
        Ok(TaskStatus {
            state: TaskState::Canceled,
            message: None,
            timestamp: None,
        })
    }
}

/// Middleware that wraps agent task processing.
///
/// Middlewares form a chain-of-responsibility pipeline. Each middleware
/// receives the [`TaskRequest`] and a reference to `next` (the inner agent or
/// the next middleware). It can inspect or modify the request, short-circuit
/// by returning early, or delegate to `next.handle_task(request)` and
/// post-process the response.
///
/// # Example — simple timing middleware
///
/// ```rust,ignore
/// use messaggero::prelude::*;
/// use std::time::Instant;
///
/// struct TimingMiddleware;
///
/// #[async_trait]
/// impl Middleware for TimingMiddleware {
///     async fn process(
///         &self,
///         request: TaskRequest,
///         next: &dyn Agent,
///     ) -> Result<TaskResponse, AgentError> {
///         let start = Instant::now();
///         let result = next.handle_task(request).await;
///         tracing::info!(elapsed_ms = start.elapsed().as_millis(), "task handled");
///         result
///     }
/// }
/// ```
#[async_trait]
pub trait Middleware: Send + Sync {
    /// Process a request, optionally delegating to `next`.
    async fn process(
        &self,
        request: TaskRequest,
        next: &dyn Agent,
    ) -> Result<TaskResponse, AgentError>;
}

/// Wraps an [`Agent`] with an ordered stack of [`Middleware`]s.
///
/// Middlewares are applied in insertion order: the first `.with()` call
/// produces the outermost layer (called first on the way in, last on the
/// way out).
///
/// # Example
///
/// ```rust,ignore
/// use messaggero::prelude::*;
///
/// let agent = MiddlewareStack::new(MyAgent)
///     .with(LoggingMiddleware)
///     .with(TimingMiddleware);
///
/// messaggero::serve(agent).fast("/tmp/my.sock").run().await?;
/// ```
pub struct MiddlewareStack {
    agent: Box<dyn Agent>,
    middlewares: Vec<Box<dyn Middleware>>,
}

impl MiddlewareStack {
    /// Create a new stack wrapping the given agent with no middlewares yet.
    pub fn new(agent: impl Agent + 'static) -> Self {
        Self {
            agent: Box::new(agent),
            middlewares: Vec::new(),
        }
    }

    /// Add a middleware layer to the stack.
    ///
    /// Layers are applied in the order they are added: the first `.with()`
    /// is the outermost layer.
    pub fn with(mut self, middleware: impl Middleware + 'static) -> Self {
        self.middlewares.push(Box::new(middleware));
        self
    }
}

#[async_trait]
impl Agent for MiddlewareStack {
    fn card(&self) -> AgentCard {
        self.agent.card()
    }

    async fn handle_task(&self, request: TaskRequest) -> Result<TaskResponse, AgentError> {
        if self.middlewares.is_empty() {
            return self.agent.handle_task(request).await;
        }

        struct ChainedAgent<'a> {
            agent: &'a dyn Agent,
            remaining: &'a [Box<dyn Middleware>],
        }

        #[async_trait]
        impl Agent for ChainedAgent<'_> {
            fn card(&self) -> AgentCard {
                self.agent.card()
            }

            async fn handle_task(&self, request: TaskRequest) -> Result<TaskResponse, AgentError> {
                if let Some((current, rest)) = self.remaining.split_first() {
                    let next = ChainedAgent {
                        agent: self.agent,
                        remaining: rest,
                    };
                    current.process(request, &next).await
                } else {
                    self.agent.handle_task(request).await
                }
            }

            async fn handle_cancel(&self, task_id: &str) -> Result<TaskStatus, AgentError> {
                self.agent.handle_cancel(task_id).await
            }
        }

        let chain = ChainedAgent {
            agent: self.agent.as_ref(),
            remaining: &self.middlewares,
        };
        chain.handle_task(request).await
    }

    async fn handle_cancel(&self, task_id: &str) -> Result<TaskStatus, AgentError> {
        self.agent.handle_cancel(task_id).await
    }
}

/// Built-in logging middleware that emits [`tracing`] events for every task.
///
/// Emits an `INFO` event when a task is received and another when it completes
/// (or an `ERROR` if it fails). Both events carry `task_id` as a structured field.
///
/// # Example
///
/// ```rust,ignore
/// use messaggero::prelude::*;
///
/// let agent = MiddlewareStack::new(MyAgent).with(LoggingMiddleware);
/// ```
pub struct LoggingMiddleware;

#[async_trait]
impl Middleware for LoggingMiddleware {
    async fn process(
        &self,
        request: TaskRequest,
        next: &dyn Agent,
    ) -> Result<TaskResponse, AgentError> {
        tracing::info!(task_id = %request.id, "handling task request");
        let result = next.handle_task(request).await;
        match &result {
            Ok(resp) => {
                tracing::info!(task_id = %resp.id, state = ?resp.status.state, "task completed");
            }
            Err(e) => {
                tracing::error!(error = %e, "task failed");
            }
        }
        result
    }
}
