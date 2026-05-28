use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("not found")]
    NotFound,

    #[error("invalid sha: {0}")]
    InvalidSha(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("storage backend error: {0}")]
    Storage(String),
}
