use std::path::Path;
use std::sync::Arc;

use crate::core::{Agent, TaskRequest, TransportError};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{UnixListener, UnixStream};
use tokio_util::codec::Framed;
use tracing::{error, info};

use super::codec::{FastCodec, FastMessage};

/// Serve an agent over a Unix domain socket using the fast binary protocol.
pub async fn serve(
    agent: Arc<dyn Agent>,
    socket_path: impl AsRef<Path>,
) -> Result<(), TransportError> {
    let path = socket_path.as_ref();

    // Clean up stale socket
    if path.exists() {
        std::fs::remove_file(path)?;
    }

    let listener = UnixListener::bind(path)?;
    info!(path = %path.display(), "fast transport listening");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let agent = agent.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(agent, stream).await {
                error!(error = %e, "connection handler error");
            }
        });
    }
}

async fn handle_connection(
    agent: Arc<dyn Agent>,
    stream: UnixStream,
) -> Result<(), TransportError> {
    let mut framed = Framed::new(stream, FastCodec);

    while let Some(result) = framed.next().await {
        let msg = result.map_err(TransportError::Codec)?;

        let response = match msg {
            FastMessage::Ping => FastMessage::Pong,

            FastMessage::TaskRequest(req) => match agent.handle_task(req).await {
                Ok(resp) => FastMessage::TaskResponse(resp),
                Err(e) => FastMessage::Error(e.to_string()),
            },

            FastMessage::Pong | FastMessage::TaskResponse(_) | FastMessage::Error(_) => {
                continue;
            }
        };

        framed.send(response).await.map_err(TransportError::Codec)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// FastClient
// ---------------------------------------------------------------------------

/// Client for the fast binary transport over Unix domain sockets.
pub struct FastClient {
    framed: Framed<UnixStream, FastCodec>,
    #[cfg(feature = "transport-log")]
    logger: Option<super::super::log::TransportLogger>,
}

impl FastClient {
    /// Connect to an agent listening on the given Unix socket path.
    pub async fn connect(socket_path: impl AsRef<Path>) -> Result<Self, TransportError> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(Self {
            framed: Framed::new(stream, FastCodec),
            #[cfg(feature = "transport-log")]
            logger: None,
        })
    }

    /// Attach a [`TransportLogger`] to record every outbound task request.
    ///
    /// Available only with the `transport-log` feature. Each `send_task` call
    /// will enqueue one [`LogEntry`](super::super::log::LogEntry) with the round-trip
    /// duration.
    ///
    /// [`TransportLogger`]: super::super::log::TransportLogger
    #[must_use]
    #[cfg(feature = "transport-log")]
    pub fn with_logger(mut self, logger: super::super::log::TransportLogger) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Attach a [`TransportLogger`] by shared reference.
    ///
    /// The logger is cloned internally (cheap: only an [`mpsc::Sender`] clone).
    /// Useful when the client was already constructed and you want to add
    /// logging afterwards (e.g. inside [`MessaggeroClient`](crate)).
    ///
    /// [`TransportLogger`]: super::super::log::TransportLogger
    /// [`mpsc::Sender`]: tokio::sync::mpsc::Sender
    #[cfg(feature = "transport-log")]
    pub fn set_logger(&mut self, logger: &super::super::log::TransportLogger) {
        self.logger = Some(logger.clone());
    }

    /// Send a task request and wait for the response.
    pub async fn send_task(&mut self, request: TaskRequest) -> Result<FastMessage, TransportError> {
        #[cfg(feature = "transport-log")]
        let (log_task_id, log_session_id, log_start) = (
            request.id.clone(),
            request.session_id.clone(),
            std::time::Instant::now(),
        );

        self.framed
            .send(FastMessage::TaskRequest(request))
            .await
            .map_err(TransportError::Codec)?;

        let result = match self.framed.next().await {
            Some(Ok(msg)) => Ok(msg),
            Some(Err(e)) => Err(TransportError::Codec(e)),
            None => Err(TransportError::Connection("connection closed".into())),
        };

        #[cfg(feature = "transport-log")]
        if let Some(ref logger) = self.logger {
            #[allow(clippy::cast_possible_truncation)]
            let duration_us = log_start.elapsed().as_micros() as u64;
            let llm_us = result.as_ref().ok().and_then(|msg| {
                if let FastMessage::TaskResponse(resp) = msg {
                    let meta = resp.metadata.as_ref()?;
                    meta.get("llm_us")
                        .and_then(|v| v.parse::<u64>().ok())
                        .or_else(|| {
                            meta.get("llm_ms")
                                .and_then(|v| v.parse::<u64>().ok())
                                .map(|ms| ms * 1_000)
                        })
                } else {
                    None
                }
            });
            let transport_us = llm_us.map(|l| duration_us.saturating_sub(l));
            let (status, error) = match &result {
                Ok(_) => ("ok", None),
                Err(e) => ("error", Some(e.to_string())),
            };
            logger.record(super::super::log::LogEntry {
                ts: super::super::log::now_iso8601(),
                transport: super::super::log::TransportKind::Fast,
                direction: super::super::log::Direction::Outbound,
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

    /// Send a ping and return `true` if a pong is received.
    pub async fn ping(&mut self) -> Result<bool, TransportError> {
        self.framed
            .send(FastMessage::Ping)
            .await
            .map_err(TransportError::Codec)?;

        match self.framed.next().await {
            Some(Ok(FastMessage::Pong)) => Ok(true),
            Some(Ok(_)) => Ok(false),
            Some(Err(e)) => Err(TransportError::Codec(e)),
            None => Err(TransportError::Connection("connection closed".into())),
        }
    }
}
