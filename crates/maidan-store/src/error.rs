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

    /// A subscribe / backfill cursor points into a gap the retention sweeper
    /// already deleted (Cluster 388). Fail loud — never clamp to the oldest
    /// remaining row. REST maps this to 409 + `must_refetch`.
    #[error(
        "cursor too old: after_id {after_id} is behind oldest retained event {oldest_id}; must refetch"
    )]
    CursorTooOld { after_id: i64, oldest_id: i64 },

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

    pub fn cursor_too_old(after_id: i64, oldest_id: i64) -> Self {
        Self::CursorTooOld {
            after_id,
            oldest_id,
        }
    }
}
