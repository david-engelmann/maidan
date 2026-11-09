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

    /// A thread write refused because, for one of the workspace's unique key
    /// sets, another open thread already gives the same values for every key
    /// of the set. Nothing was written. `held` is the set with this write's
    /// values, in the set's order. REST 409 `conflict` and MCP the same,
    /// naming `holder` only to a caller who may read it, so the refusal never
    /// discloses a thread in a channel the caller cannot see.
    #[error(
        "{} is already held by an open thread in this workspace; nothing was written. Work in that thread, or close it first",
        maidan_types::describe_held(.held)
    )]
    AttributeHeld {
        held: Vec<(maidan_types::UniqueAttributeKey, String)>,
        holder: maidan_types::ThreadId,
    },

    /// A claim-fenced write from a member that no longer holds the thread
    /// under the lease it named: the lease expired (taken over or not), or
    /// another member took the thread, or the claim was released. Nothing was
    /// written. REST 409 `claim-lost`, MCP its own error code with
    /// `data.code` `claim_lost`.
    #[error("claim lost: {0}")]
    ClaimLost(String),

    /// A write aimed at a thread that is gone: tombstoned, kept only as a
    /// marker. Nothing was written. REST 410 `gone`, MCP its own error code
    /// with `data.code` `gone`.
    #[error("gone: {0}")]
    Gone(String),

    /// A write the thread's ended state refuses: it is closed or archived,
    /// as the write's own rule says. Nothing was written. REST 409
    /// `thread-ended`, MCP its own error code with `data.code`
    /// `thread_ended`.
    #[error("thread ended: {0}")]
    Ended(String),

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
