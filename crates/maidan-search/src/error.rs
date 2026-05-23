use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error("operation not supported by backend: {0}")]
    Unsupported(&'static str),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}
