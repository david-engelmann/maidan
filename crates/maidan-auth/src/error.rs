use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("missing or invalid bearer token")]
    Unauthorized,

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("database error: {0}")]
    Store(#[from] maidan_store::StoreError),
}
