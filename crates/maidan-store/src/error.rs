use maidan_types::SpawnDenial;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,

    #[error("uniqueness violation: {0}")]
    Conflict(String),

    /// A spawn refused by the workspace's spawn budget. A [`Conflict`] by wire
    /// behaviour — REST 409 / MCP InvalidParams, with the denial's `Display` as
    /// the message — but typed, so the caller can publish the
    /// `ThreadSpawnDenied` event without parsing a string.
    ///
    /// [`Conflict`]: StoreError::Conflict
    #[error("{0}")]
    SpawnRejected(Box<SpawnDenial>),

    /// A subscribe / backfill cursor points into a gap the retention sweeper
    /// already deleted. Fail loud — never clamp to the oldest remaining row.
    /// REST maps this to 409 + `must_refetch`.
    #[error(
        "cursor too old: after_id {after_id} is behind oldest retained event {oldest_id}; must refetch"
    )]
    CursorTooOld { after_id: i64, oldest_id: i64 },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// A claim-fenced write from a member that no longer holds the thread
    /// under the lease it named: the lease lapsed and another member took
    /// the thread, or the claim was released. Nothing was written. REST 409
    /// `claim-lost`, MCP InvalidParams.
    #[error("claim lost: {0}")]
    ClaimLost(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// A content key could not be wrapped or unwrapped, or sealed words failed
    /// authentication. A KEK missing from the configuration lands here, never
    /// as a shredded key.
    #[error("content key error: {0}")]
    ContentKey(#[from] maidan_types::SealError),

    /// A write refused because its workspace is an open sign-up workspace
    /// past its free allowance on this meter this month. REST 429, MCP
    /// `OverAllowance`. Raised at the store's message and artifact writes,
    /// so every writer is held to it, not only the routes that list it.
    #[error("this workspace is past its free allowance for {} this month", .0.as_str())]
    OverAllowance(maidan_types::UsageMeter),
}

impl StoreError {
    /// Whether the key custody refused this call for good (as opposed to a
    /// transient custody failure worth asking again in the same pass).
    pub fn is_custody_refusal(&self) -> bool {
        matches!(self, Self::ContentKey(maidan_types::SealError::Refused))
    }

    /// Build a [`SpawnRejected`]. The denial is boxed so the variant doesn't
    /// inflate every `StoreError` on the hot path.
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
