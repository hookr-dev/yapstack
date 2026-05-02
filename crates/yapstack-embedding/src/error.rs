use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("sidecar exited unexpectedly")]
    SidecarDead,

    #[error("sidecar reported error: {0}")]
    SidecarError(String),

    #[error("response channel closed before reply")]
    ResponseDropped,

    #[error("timed out waiting for response")]
    Timeout,
}
