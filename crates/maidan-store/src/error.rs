use maidan_types::SpawnDenial;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,

    #[error("uniqueness violation: {0}")]
    Conflict(String),

    /// A spawn refused by the workspace's spawn budget (Cluster 376). A
    /// [`Conflict`] by wire behaviour — REST 409 / MCP InvalidParams, with the
    /// denial's `Display` as the message — but typed, so the caller can publish
    /// the `ThreadSpawnDenied` event without parsing a string.
    ///
    /// [`Conflict`]: StoreError::Conflict
    #[error("{0}")]
    SpawnRejected(Box<SpawnDenial>),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl StoreError {
    /// Build a [`SpawnRejected`] (Cluster 376.6). The denial is boxed so the
    /// variant doesn't inflate every `StoreError` on the hot path.
    ///
    /// [`SpawnRejected`]: StoreError::SpawnRejected
    pub fn spawn_rejected(denial: SpawnDenial) -> Self {
        Self::SpawnRejected(Box::new(denial))
    }
}
