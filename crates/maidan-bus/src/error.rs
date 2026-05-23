use thiserror::Error;

#[derive(Debug, Error)]
pub enum BusError {
    #[error("payload too large for backend: {0} bytes")]
    PayloadTooLarge(usize),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("bus closed")]
    Closed,
}
