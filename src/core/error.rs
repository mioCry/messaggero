use thiserror::Error;

#[non_exhaustive]
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("agent internal error: {0}")]
    Internal(String),

    #[error("task canceled: {0}")]
    Canceled(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

#[non_exhaustive]
#[derive(Debug, Error)]
pub enum CodecError {
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("binary serialization error: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("invalid frame: {0}")]
    InvalidFrame(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[non_exhaustive]
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("connection error: {0}")]
    Connection(String),

    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("codec error: {0}")]
    Codec(#[from] CodecError),

    #[error("agent error: {0}")]
    Agent(#[from] AgentError),

    #[error("request error: {0}")]
    Request(String),

    #[error("timeout")]
    Timeout,
}
