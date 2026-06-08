use messaggero_core::jsonrpc::*;
use messaggero_core::{AgentCard, TaskRequest, TaskResponse, TransportError};

/// HTTP client for A2A-compatible JSON-RPC communication.
pub struct A2AClient {
    base_url: String,
    http: reqwest::Client,
    #[cfg(feature = "transport-log")]
    logger: Option<crate::log::TransportLogger>,
}

impl A2AClient {
    /// Create a new client pointing at the given base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
            #[cfg(feature = "transport-log")]
            logger: None,
        }
    }

    /// Attach a [`TransportLogger`] to record every outbound task request.
    ///
    /// Available only with the `transport-log` feature. Each `send_task` call
    /// will enqueue one [`LogEntry`](crate::log::LogEntry) with the round-trip
    /// duration.
    ///
    /// [`TransportLogger`]: crate::log::TransportLogger
    #[must_use]
    #[cfg(feature = "transport-log")]
    pub fn with_logger(mut self, logger: crate::log::TransportLogger) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Attach a [`TransportLogger`] by shared reference.
    ///
    /// The logger is cloned internally (cheap: only an `mpsc::Sender` clone).
    /// Useful when the client is already constructed and you want to add
    /// logging afterwards (e.g. inside `MessaggeroClient`).
    #[cfg(feature = "transport-log")]
    pub fn set_logger(&mut self, logger: &crate::log::TransportLogger) {
        self.logger = Some(logger.clone());
    }

    /// Fetch the remote agent's capability card.
    pub async fn agent_card(&self) -> Result<AgentCard, TransportError> {
        let url = format!("{}/.well-known/agent.json", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| TransportError::Request(e.to_string()))?;

        let card: AgentCard = resp
            .json()
            .await
            .map_err(|e| TransportError::Request(e.to_string()))?;

        Ok(card)
    }

    /// Send a task request to the remote agent (A2A `tasks/send`).
    pub async fn send_task(&self, request: TaskRequest) -> Result<TaskResponse, TransportError> {
        #[cfg(feature = "transport-log")]
        let (log_task_id, log_session_id, log_start) = (
            request.id.clone(),
            request.session_id.clone(),
            std::time::Instant::now(),
        );

        let result = self.do_send_task(request).await;

        #[cfg(feature = "transport-log")]
        if let Some(ref logger) = self.logger {
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
            let (status, error) = match &result {
                Ok(_) => ("ok", None),
                Err(e) => ("error", Some(e.to_string())),
            };
            logger.record(crate::log::LogEntry {
                ts: crate::log::now_iso8601(),
                transport: crate::log::TransportKind::A2a,
                direction: crate::log::Direction::Outbound,
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

        result
    }

    async fn do_send_task(&self, request: TaskRequest) -> Result<TaskResponse, TransportError> {
        let params =
            serde_json::to_value(&request).map_err(|e| TransportError::Request(e.to_string()))?;

        let rpc_request = JsonRpcRequest::new(METHOD_TASKS_SEND, params);

        let rpc_response: JsonRpcResponse = self
            .http
            .post(&self.base_url)
            .json(&rpc_request)
            .send()
            .await
            .map_err(|e| TransportError::Request(e.to_string()))?
            .json()
            .await
            .map_err(|e| TransportError::Request(e.to_string()))?;

        if let Some(err) = rpc_response.error {
            return Err(TransportError::Request(format!(
                "JSON-RPC error {}: {}",
                err.code, err.message
            )));
        }

        let result = rpc_response
            .result
            .ok_or_else(|| TransportError::Request("empty result".into()))?;

        let task_response: TaskResponse =
            serde_json::from_value(result).map_err(|e| TransportError::Request(e.to_string()))?;

        Ok(task_response)
    }

    /// Cancel a running task on the remote agent.
    pub async fn cancel_task(&self, task_id: &str) -> Result<(), TransportError> {
        let params = serde_json::json!({ "id": task_id });
        let rpc_request = JsonRpcRequest::new(METHOD_TASKS_CANCEL, params);

        let rpc_response: JsonRpcResponse = self
            .http
            .post(&self.base_url)
            .json(&rpc_request)
            .send()
            .await
            .map_err(|e| TransportError::Request(e.to_string()))?
            .json()
            .await
            .map_err(|e| TransportError::Request(e.to_string()))?;

        if let Some(err) = rpc_response.error {
            return Err(TransportError::Request(format!(
                "JSON-RPC error {}: {}",
                err.code, err.message
            )));
        }

        Ok(())
    }
}
