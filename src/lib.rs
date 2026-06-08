#![deny(missing_docs, clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc
)]

//! # Messaggero
//!
//! High-performance AI agent communication protocol for Rust.
//!
//! Provides A2A-compatible interoperability over HTTP/JSON-RPC alongside a
//! fast binary transport over Unix domain sockets for local agent-to-agent
//! communication with minimal overhead.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use messaggero::prelude::*;
//!
//! struct EchoAgent;
//!
//! #[async_trait]
//! impl Agent for EchoAgent {
//!     fn card(&self) -> AgentCard {
//!         AgentCard::builder("echo")
//!             .description("Echoes messages back")
//!             .skill("echo", "Echo", "Echoes any message")
//!             .build()
//!     }
//!
//!     async fn handle_task(&self, req: TaskRequest) -> Result<TaskResponse, AgentError> {
//!         let text = req.message.text_content().unwrap_or("...");
//!         Ok(TaskResponse::completed(&req.id, Message::agent(format!("Echo: {text}"))))
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     messaggero::serve(EchoAgent)
//!         .fast("/tmp/echo.sock")
//!         .http("127.0.0.1:3000")
//!         .run()
//!         .await
//! }
//! ```

pub use messaggero_core as core;
pub use messaggero_transport as transport;

// Procedural macros re-exported so users never need to name messaggero-macros directly.
pub use messaggero_macros::AgentCard as DeriveAgentCard;

pub use messaggero_core::{
    agent::{Agent, LoggingMiddleware, Middleware, MiddlewareStack},
    codec::Encoding,
    error::{AgentError, CodecError, TransportError},
    jsonrpc,
    types::*,
};

pub use messaggero_transport::{AgentEndpoint, Discovery, Router};

#[cfg(feature = "fast")]
pub use messaggero_transport::fast::{FastClient, FastMessage};

#[cfg(feature = "a2a")]
pub use messaggero_transport::a2a::A2AClient;

pub use async_trait::async_trait;

// Transport audit logger — only available with the `transport-log` feature.
#[cfg(feature = "transport-log")]
pub use messaggero_transport::log::{
    Direction, LogEntry, TransportKind, TransportLogger, TransportLoggerBuilder,
};

/// Convenience re-exports for `use messaggero::prelude::*`.
///
/// Import everything you need to implement and serve an agent:
///
/// ```rust,ignore
/// use messaggero::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        Agent, AgentCard, AgentError, Artifact, Message, Middleware, MiddlewareStack, Part, Role,
        Task, TaskRequest, TaskResponse, TaskState, TaskStatus,
    };
    pub use async_trait::async_trait;
    // `#[derive(DeriveAgentCard)]` available without naming the macros crate.
    pub use messaggero_macros::AgentCard as DeriveAgentCard;
}

// ---------------------------------------------------------------------------
// Server-side logging shim (inbound task recording)
// ---------------------------------------------------------------------------

/// Internal enum used to select a transport-specific [`TransportLogger`] shim.
///
/// Kept as a regular (non-cfg-gated) enum so that [`ServerBuilder::make_agent`]
/// compiles without `#[cfg]` on function parameters (unsupported in MSRV 1.75).
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum TransportContext {
    Fast,
    A2a,
}

/// Transparent [`Agent`] wrapper that records every inbound task request.
///
/// Created internally by [`ServerBuilder`] when a logger is attached. Each
/// transport gets its own `LoggedAgent` instance so that the `transport` field
/// in the log entry correctly identifies which protocol was used.
#[cfg(feature = "transport-log")]
struct LoggedAgent {
    inner: std::sync::Arc<dyn Agent>,
    logger: messaggero_transport::log::TransportLogger,
    transport: messaggero_transport::log::TransportKind,
}

#[cfg(feature = "transport-log")]
#[async_trait]
impl Agent for LoggedAgent {
    fn card(&self) -> AgentCard {
        self.inner.card()
    }

    async fn handle_task(&self, request: TaskRequest) -> Result<TaskResponse, AgentError> {
        let start = std::time::Instant::now();
        let task_id = request.id.clone();
        let session_id = request.session_id.clone();
        let result = self.inner.handle_task(request).await;
        // Microseconds since the start; u128 → u64 truncation is safe in practice
        // (u64::MAX microseconds ≈ 584 000 years).
        #[allow(clippy::cast_possible_truncation)]
        let duration_us = start.elapsed().as_micros() as u64;
        let (status, error) = match &result {
            Ok(_) => ("ok", None),
            Err(e) => ("error", Some(e.to_string())),
        };
        self.logger.record(messaggero_transport::log::LogEntry {
            ts: messaggero_transport::log::now_iso8601(),
            transport: self.transport,
            direction: messaggero_transport::log::Direction::Inbound,
            task_id,
            session_id,
            duration_us,
            llm_us: None,
            transport_us: None,
            status,
            error,
            payload_bytes: None,
        });
        result
    }

    async fn handle_cancel(&self, task_id: &str) -> Result<TaskStatus, AgentError> {
        self.inner.handle_cancel(task_id).await
    }
}

// ---------------------------------------------------------------------------
// Server builder
// ---------------------------------------------------------------------------

/// Builder for serving an agent on one or more transports simultaneously.
///
/// Obtain via [`serve`].
// fast_path and http_addr are read inside #[cfg(feature = "fast"/"a2a")] blocks;
// when both features are off the fields are genuinely unused. Suppressed here to
// keep --no-default-features builds warning-free.
#[allow(dead_code)]
pub struct ServerBuilder {
    agent: std::sync::Arc<dyn Agent>,
    fast_path: Option<String>,
    http_addr: Option<String>,
    #[cfg(feature = "transport-log")]
    logger: Option<messaggero_transport::log::TransportLogger>,
}

/// Create a [`ServerBuilder`] to serve the given agent.
pub fn serve(agent: impl Agent + 'static) -> ServerBuilder {
    ServerBuilder {
        agent: std::sync::Arc::new(agent),
        fast_path: None,
        http_addr: None,
        #[cfg(feature = "transport-log")]
        logger: None,
    }
}

impl ServerBuilder {
    /// Enable the fast binary transport on the given Unix socket path.
    #[must_use]
    #[cfg(feature = "fast")]
    pub fn fast(mut self, socket_path: impl Into<String>) -> Self {
        self.fast_path = Some(socket_path.into());
        self
    }

    /// Enable the A2A HTTP transport on the given address (e.g. `"127.0.0.1:3000"`).
    #[must_use]
    #[cfg(feature = "a2a")]
    pub fn http(mut self, addr: impl Into<String>) -> Self {
        self.http_addr = Some(addr.into());
        self
    }

    /// Attach a [`TransportLogger`] to record every inbound task request.
    ///
    /// Available only with the `transport-log` feature (disabled by default).
    ///
    /// When set, each transport receives a transparent [`LoggedAgent`] shim
    /// that measures handler latency and writes one
    /// [`LogEntry`](messaggero_transport::log::LogEntry) per task. The
    /// `transport` field in the log correctly identifies whether the call
    /// arrived via the fast binary path (`"fast"`) or A2A HTTP (`"a2a"`).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use messaggero::TransportLogger;
    ///
    /// let logger = TransportLogger::builder()
    ///     .log_dir("/var/log/myapp/transport")
    ///     .max_entries(1000)
    ///     .build()
    ///     .await?;
    ///
    /// messaggero::serve(MyAgent)
    ///     .fast("/tmp/agent.sock")
    ///     .http("127.0.0.1:3000")
    ///     .with_transport_logger(logger)
    ///     .run()
    ///     .await?;
    /// ```
    #[must_use]
    #[cfg(feature = "transport-log")]
    pub fn with_transport_logger(
        mut self,
        logger: messaggero_transport::log::TransportLogger,
    ) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Start all configured transports and block until the **first** one exits.
    ///
    /// All transports are spawned as independent Tokio tasks and run concurrently.
    /// As soon as any single transport terminates — either because it received a
    /// shutdown signal, encountered an I/O error, or completed normally — this
    /// method returns and the remaining transports are abandoned (their tasks are
    /// dropped by the runtime).
    ///
    /// This means that a transient error on one transport (e.g. the Unix socket
    /// listener fails) will take down the whole server. For production deployments
    /// that need independent restart logic per transport, manage the transports
    /// yourself by calling [`messaggero_transport::fast::serve`] and
    /// [`messaggero_transport::a2a::serve`] directly.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No transports were configured before calling `run`.
    /// - A spawned transport task panics (join error).
    /// - The first transport to finish returns a [`TransportError`].
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        #[allow(unused_mut)]
        let mut handles: Vec<tokio::task::JoinHandle<Result<(), TransportError>>> = Vec::new();

        #[cfg(feature = "fast")]
        if let Some(ref path) = self.fast_path {
            let agent = self.make_agent(TransportContext::Fast);
            let path = path.clone();
            handles.push(tokio::spawn(async move {
                messaggero_transport::fast::serve(agent, path).await
            }));
        }

        #[cfg(feature = "a2a")]
        if let Some(ref addr) = self.http_addr {
            let agent = self.make_agent(TransportContext::A2a);
            let addr = addr.clone();
            handles.push(tokio::spawn(async move {
                messaggero_transport::a2a::serve(agent, addr).await
            }));
        }

        if handles.is_empty() {
            return Err("no transports configured — call .fast() and/or .http()".into());
        }

        // Wait for any transport to finish (or error out)
        let (result, _idx, _remaining) = futures_util::future::select_all(handles).await;
        result??;

        Ok(())
    }

    /// Wrap the agent in a `LoggedAgent` shim when a logger is configured,
    /// or return a plain clone otherwise.
    ///
    /// The `_context` parameter is used only when `transport-log` is enabled;
    /// the leading underscore suppresses the unused-variable warning in builds
    /// where the feature is absent.
    // `context` is used inside the #[cfg(feature = "transport-log")] block; when
    // that feature is disabled it appears unused, hence the allow attribute.
    #[allow(dead_code, unused_variables)]
    fn make_agent(&self, context: TransportContext) -> std::sync::Arc<dyn Agent> {
        #[cfg(feature = "transport-log")]
        if let Some(ref logger) = self.logger {
            let transport = match context {
                TransportContext::Fast => messaggero_transport::log::TransportKind::Fast,
                TransportContext::A2a => messaggero_transport::log::TransportKind::A2a,
            };
            return std::sync::Arc::new(LoggedAgent {
                inner: self.agent.clone(),
                logger: logger.clone(),
                transport,
            });
        }
        self.agent.clone()
    }
}

// ---------------------------------------------------------------------------
// Unified client
// ---------------------------------------------------------------------------

/// Unified client that auto-selects the best transport for reaching an agent.
///
/// # Transport logging
///
/// When the `transport-log` feature is enabled, call
/// [`with_transport_logger`](Self::with_transport_logger) after connecting to
/// attach a [`TransportLogger`]. Every subsequent `send_task` call will then
/// record a log entry.
pub struct MessaggeroClient {
    #[cfg(feature = "fast")]
    fast: Option<FastClient>,
    #[cfg(feature = "a2a")]
    a2a: Option<A2AClient>,
}

impl MessaggeroClient {
    /// Connect to a local agent via the fast Unix socket transport.
    #[cfg(feature = "fast")]
    pub async fn connect_fast(
        socket_path: impl AsRef<std::path::Path>,
    ) -> Result<Self, TransportError> {
        let client = FastClient::connect(socket_path).await?;
        Ok(Self {
            fast: Some(client),
            #[cfg(feature = "a2a")]
            a2a: None,
        })
    }

    /// Connect to a remote agent via the A2A HTTP transport.
    #[cfg(feature = "a2a")]
    pub fn connect_http(base_url: impl Into<String>) -> Self {
        Self {
            #[cfg(feature = "fast")]
            fast: None,
            a2a: Some(A2AClient::new(base_url)),
        }
    }

    /// Attach a [`TransportLogger`] to record every outbound task request.
    ///
    /// Available only with the `transport-log` feature. The logger is
    /// forwarded to the underlying [`FastClient`] and/or [`A2AClient`]
    /// depending on which transports are connected.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let logger = TransportLogger::builder()
    ///     .log_dir("/var/log/myapp/transport")
    ///     .build()
    ///     .await?;
    ///
    /// let client = MessaggeroClient::connect_fast("/tmp/agent.sock")
    ///     .await?
    ///     .with_transport_logger(logger);
    ///
    /// client.send_task(request).await?;
    /// ```
    // TransportLogger is a cheap mpsc::Sender clone; passing by reference and
    // cloning internally in set_logger avoids both needless_pass_by_value and
    // the redundant-clone that arises from distributing to two inner clients.
    #[must_use]
    #[cfg(feature = "transport-log")]
    pub fn with_transport_logger(
        mut self,
        logger: &messaggero_transport::log::TransportLogger,
    ) -> Self {
        #[cfg(feature = "fast")]
        if let Some(ref mut fc) = self.fast {
            fc.set_logger(logger);
        }
        #[cfg(feature = "a2a")]
        if let Some(ref mut a2a) = self.a2a {
            a2a.set_logger(logger);
        }
        self
    }

    /// Send a task request to the connected agent.
    pub async fn send_task(
        &mut self,
        request: TaskRequest,
    ) -> Result<TaskResponse, TransportError> {
        // Suppress unused-variable warning when both transports are disabled.
        #[cfg(not(any(feature = "fast", feature = "a2a")))]
        let _ = request;

        #[cfg(feature = "fast")]
        if let Some(ref mut fast) = self.fast {
            let msg = fast.send_task(request).await?;
            return match msg {
                FastMessage::TaskResponse(resp) => Ok(resp),
                FastMessage::Error(e) => Err(TransportError::Request(e)),
                other => Err(TransportError::Request(format!(
                    "unexpected response: {other:?}"
                ))),
            };
        }

        #[cfg(feature = "a2a")]
        if let Some(ref a2a) = self.a2a {
            return a2a.send_task(request).await;
        }

        Err(TransportError::Connection("no transport configured".into()))
    }

    /// Fetch the remote agent's capability card (A2A only).
    #[cfg(feature = "a2a")]
    pub async fn agent_card(&self) -> Result<AgentCard, TransportError> {
        self.a2a
            .as_ref()
            .ok_or_else(|| TransportError::Connection("not connected via HTTP".into()))?
            .agent_card()
            .await
    }
}
