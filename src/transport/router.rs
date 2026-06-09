use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::core::{TaskRequest, TaskResponse, TransportError};
use tokio::sync::RwLock;

#[cfg(feature = "fast")]
use super::fast;

#[cfg(feature = "a2a")]
use super::a2a;

/// Describes how to reach a specific agent.
#[derive(Debug, Clone)]
pub enum AgentEndpoint {
    /// Local Unix socket path — will use the fast binary transport.
    Fast(PathBuf),
    /// Remote HTTP URL — will use the A2A JSON-RPC transport.
    Http(String),
}

/// Smart router that automatically selects the best transport for each agent.
///
/// Prefers the fast binary path for local agents and falls back to A2A HTTP
/// for remote ones. Agents are registered by name.
///
/// # Transport logging
///
/// When the `transport-log` feature is enabled, call
/// [`with_transport_logger`](Self::with_transport_logger) to attach a
/// [`TransportLogger`](super::log::TransportLogger). Every `send` call will
/// then record a [`LogEntry`](super::log::LogEntry) with the round-trip
/// duration and the transport kind actually used.
pub struct Router {
    endpoints: Arc<RwLock<HashMap<String, AgentEndpoint>>>,
    #[cfg(feature = "transport-log")]
    logger: Option<super::log::TransportLogger>,
}

impl Router {
    /// Create a new empty router.
    pub fn new() -> Self {
        Self {
            endpoints: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "transport-log")]
            logger: None,
        }
    }

    /// Attach a [`TransportLogger`] to record every outbound task dispatch.
    ///
    /// Available only with the `transport-log` feature. The logger records
    /// one entry per `send` call with the correct [`TransportKind`] (fast or
    /// a2a) and the full round-trip duration.
    ///
    /// [`TransportLogger`]: super::log::TransportLogger
    /// [`TransportKind`]: super::log::TransportKind
    #[must_use]
    #[cfg(feature = "transport-log")]
    pub fn with_transport_logger(mut self, logger: super::log::TransportLogger) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Register an agent endpoint.
    pub async fn register(&self, name: impl Into<String>, endpoint: AgentEndpoint) {
        self.endpoints.write().await.insert(name.into(), endpoint);
    }

    /// Remove an agent endpoint.
    pub async fn unregister(&self, name: &str) {
        self.endpoints.write().await.remove(name);
    }

    /// List all registered agent names.
    pub async fn agents(&self) -> Vec<String> {
        self.endpoints.read().await.keys().cloned().collect()
    }

    #[cfg(feature = "transport-log")]
    fn record_outbound(
        logger: &super::log::TransportLogger,
        log_start: std::time::Instant,
        log_task_id: String,
        log_session_id: Option<String>,
        transport: super::log::TransportKind,
        result: &Result<TaskResponse, TransportError>,
    ) {
        #[allow(clippy::cast_possible_truncation)]
        let duration_us = log_start.elapsed().as_micros() as u64;
        let llm_us = result.as_ref().ok().and_then(|resp| {
            let meta = resp.metadata.as_ref()?;
            meta.get("llm_us")
                .and_then(|v| v.parse::<u64>().ok())
                .or_else(|| {
                    meta.get("llm_ms")
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(|ms| ms * 1_000)
                })
        });
        let transport_us = llm_us.map(|l| duration_us.saturating_sub(l));
        let (status, error) = match result {
            Ok(_) => ("ok", None),
            Err(e) => ("error", Some(e.to_string())),
        };
        logger.record(super::log::LogEntry {
            ts: super::log::now_iso8601(),
            transport,
            direction: super::log::Direction::Outbound,
            task_id: log_task_id,
            session_id: log_session_id,
            duration_us,
            llm_us,
            transport_us,
            status,
            error,
            payload_bytes: None,
        });
    }

    /// Send a task request to a named agent, automatically choosing the transport.
    pub async fn send(
        &self,
        agent_name: &str,
        request: TaskRequest,
    ) -> Result<TaskResponse, TransportError> {
        // Start the timer before any async work (RwLock, endpoint lookup) so
        // that `duration_us` in the log matches the caller-side elapsed time.
        #[cfg(feature = "transport-log")]
        let (log_task_id, log_session_id, log_start) = (
            request.id.clone(),
            request.session_id.clone(),
            std::time::Instant::now(),
        );

        let endpoints = self.endpoints.read().await;
        let endpoint = endpoints
            .get(agent_name)
            .ok_or_else(|| TransportError::AgentNotFound(agent_name.to_string()))?
            .clone();
        drop(endpoints);

        match endpoint {
            #[cfg(feature = "fast")]
            AgentEndpoint::Fast(path) => {
                let mut client = fast::FastClient::connect(&path).await?;
                let result = match client.send_task(request).await {
                    Ok(fast::FastMessage::TaskResponse(resp)) => Ok(resp),
                    Ok(fast::FastMessage::Error(e)) => Err(TransportError::Request(e)),
                    Ok(other) => Err(TransportError::Request(format!(
                        "unexpected response: {other:?}"
                    ))),
                    Err(e) => Err(e),
                };

                #[cfg(feature = "transport-log")]
                if let Some(ref logger) = self.logger {
                    Self::record_outbound(
                        logger,
                        log_start,
                        log_task_id,
                        log_session_id,
                        super::log::TransportKind::Fast,
                        &result,
                    );
                }

                result
            }

            #[cfg(not(feature = "fast"))]
            AgentEndpoint::Fast(_) => Err(TransportError::Request(
                "fast transport not enabled (compile with feature 'fast')".into(),
            )),

            #[cfg(feature = "a2a")]
            AgentEndpoint::Http(url) => {
                let client = a2a::A2AClient::new(&url);
                let result = client.send_task(request).await;

                #[cfg(feature = "transport-log")]
                if let Some(ref logger) = self.logger {
                    Self::record_outbound(
                        logger,
                        log_start,
                        log_task_id,
                        log_session_id,
                        super::log::TransportKind::A2a,
                        &result,
                    );
                }

                result
            }

            #[cfg(not(feature = "a2a"))]
            AgentEndpoint::Http(_) => Err(TransportError::Request(
                "A2A transport not enabled (compile with feature 'a2a')".into(),
            )),
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}
