//! Storage abstraction for Maidan.
//!
//! Defines [`Store`], a backend-agnostic async interface, plus Postgres
//! and SQLite implementations backed by `sqlx`.

pub mod a2a;
pub mod attribution;

/// The audit row an authority-changing store call writes in its own
/// transaction (D-A), built from what the call produced — a new token's id,
/// say — so the row commits or rolls back with the change it records.
pub type AuditFor<T> = Box<dyn FnOnce(&T) -> maidan_types::NewAuditEvent + Send>;
/// Why a destructive call refused a workspace under legal hold. The store
/// checks inside the destroying transaction, so every caller is bound by it.
pub const LEGAL_HOLD_REFUSAL: &str =
    "workspace is under a legal hold; lift it before deleting workspace data";

/// What lifting a legal hold disposed of: the withdrawn messages it had kept,
/// and their earlier versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldDisposal {
    pub withdrawn_messages: u64,
    pub edit_versions: u64,
}

impl HoldDisposal {
    /// The lift's audit row, carrying what the lift disposed of.
    pub(crate) fn recorded_in(
        &self,
        mut event: maidan_types::NewAuditEvent,
    ) -> maidan_types::NewAuditEvent {
        let disposed = serde_json::json!({
            "withdrawn_messages": self.withdrawn_messages,
            "edit_versions": self.edit_versions,
        });
        match &mut event.metadata {
            serde_json::Value::Object(map) => {
                map.insert("disposed".into(), disposed);
            }
            other => *other = serde_json::json!({ "disposed": disposed }),
        }
        event
    }
}
/// Deletes one artifact's bytes from the blob store. Run by
/// [`store::ArtifactMetaStore::reap_artifact_blob`] while it holds the sha's
/// reap lease.
pub type BlobDelete<'a> = Box<
    dyn FnOnce() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>,
        > + Send
        + 'a,
>;

/// What [`store::ArtifactMetaStore::reap_artifact_blob`] did with a blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobReap {
    /// An artifact row holds the sha again; the bytes stay.
    Referenced,
    /// No row held the sha and the bytes were deleted.
    Deleted,
    /// No row held the sha, but deleting the bytes failed.
    DeleteFailed(String),
}

/// How long a reap's blob delete may run before the reap stops waiting for it.
pub const BLOB_DELETE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a reap's lease on a sha lasts: twice [`BLOB_DELETE_TIMEOUT`], so a
/// delete that overran has been abandoned well before the lease lapses and an
/// upload of the same bytes may go ahead. A reaper that crashes mid-delete
/// holds the sha no longer than this.
pub(crate) const REAP_LEASE_SECS: i64 = 60;

/// How often an upload, or a second reap, checks whether a reap lease on its
/// sha has gone.
pub(crate) const REAP_POLL: std::time::Duration = std::time::Duration::from_millis(25);

/// Run a reap's delete, bounded by [`BLOB_DELETE_TIMEOUT`]. `None` means it
/// overran and was abandoned; it may still land, so its lease must be left to
/// lapse rather than released.
pub(crate) async fn run_blob_delete(delete: BlobDelete<'_>) -> Option<BlobReap> {
    match tokio::time::timeout(BLOB_DELETE_TIMEOUT, delete()).await {
        Ok(Ok(())) => Some(BlobReap::Deleted),
        Ok(Err(err)) => Some(BlobReap::DeleteFailed(err)),
        Err(_) => None,
    }
}

/// What a reap reports for a delete [`run_blob_delete`] abandoned.
pub(crate) fn abandoned_delete() -> BlobReap {
    BlobReap::DeleteFailed(format!(
        "the blob delete did not finish within {}s",
        BLOB_DELETE_TIMEOUT.as_secs()
    ))
}

/// Why a review-requirement write was refused: it would lower the requirement
/// and the caller may not. Checked in the write's transaction.
pub const REVIEW_LOWER_REFUSAL: &str =
    "lowering a review requirement needs the channel:admin capability";
pub mod automation_deliveries;
mod claim_next;
pub mod content_keyring;
mod content_keys;
mod delegation_grants;
pub mod dialect;
pub mod dm;
pub mod embeddings_purge;
pub mod error;
pub mod group_dm;
pub mod idempotency;
pub mod lag_resume;
pub mod log_snapshot;
pub mod mcp_subscriptions;
pub mod migrate;
pub mod outbox;
pub mod postgres;
pub mod result_delivery;
pub mod retention_policy;
mod share_tickets;
pub mod shred_residue;
pub mod sqlite;
pub mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod thread_access;
pub mod trace;
pub mod workspace_export;

pub use a2a::{A2aPushConfigRow, A2aTaskQuery, A2aTaskRow, A2aTaskWrite, PendingGateQuery};
pub use automation_deliveries::AutomationDeliveryFilter;
pub use dialect::Dialect;
pub use error::StoreError;
pub use idempotency::{IdempotencyReservation, NewIdempotencyKey, StoredResponse};
pub use lag_resume::{resume_from_log, LAG_RESUME_BATCH};
pub use log_snapshot::{build_log_snapshot, catch_up_since, CATCH_UP_LIMIT};
pub use mcp_subscriptions::{McpSubscriptionWatch, NewMcpSubscription};
pub use migrate::{run_postgres_migrations, run_sqlite_migrations};
pub use outbox::OutboxBackend;
pub use postgres::outbox::{OutboxRow, QuarantinedOutboxRow};
pub use postgres::PostgresStore;
pub use result_delivery::{replay_result_delivery, ResultDeliveryReplay};
pub use shred_residue::{find_shred_residue_postgres, find_shred_residue_sqlite, ShredResidue};
pub use sqlite::SqliteStore;

/// Default max connections for a **file-backed SQLite** pool.
///
/// SQLite allows only one writer at a time, and sqlx's `pool.begin()` opens a
/// *deferred* transaction: with more than one pooled connection, two writers can
/// each take a read snapshot and then race to upgrade to the writer, which is a
/// genuine deadlock that `busy_timeout` cannot resolve (it returns
/// `SQLITE_BUSY` immediately rather than waiting). A contention test showed a
/// warm 8-connection pool failing ~90% of read-modify-write transactions with
/// "database is locked", while a single connection is clean. So the SQLite
/// backend serializes through one connection by default (overridable via
/// `MAIDAN_DB_MAX_CONNECTIONS` for anyone who has arranged writes to avoid the
/// upgrade deadlock). Postgres, the production/HA backend, is unaffected and
/// keeps its multi-connection pool.
pub const DEFAULT_SQLITE_MAX_CONNECTIONS: u32 = 1;

/// Applies SQLite PRAGMAs (`foreign_keys`, WAL, 5000 ms `busy_timeout`).
pub async fn configure_sqlite_pool(pool: &sqlx::SqlitePool) -> Result<(), StoreError> {
    sqlite::configure_pool(pool).await
}

/// As [`configure_sqlite_pool`], with a configurable `busy_timeout` in ms.
pub async fn configure_sqlite_pool_with(
    pool: &sqlx::SqlitePool,
    busy_timeout_ms: u64,
) -> Result<(), StoreError> {
    sqlite::configure_pool_with(pool, busy_timeout_ms).await
}
pub use store::Store;
pub use workspace_export::build_workspace_export;
// The domain sub-traits `Store` composes. Re-exported so a caller that needs
// only one concern can bound on the narrower trait; `dyn Store` still exposes
// them all via the super-trait.
pub use store::{
    A2aStore, AppStore, ArtifactMetaStore, AssignmentStore, AutomationStore, ChannelStore,
    DelegationGrantStore, DeliveryCursorStore, DmStore, EventStore, FollowStore, FsmHookStore,
    GlossaryStore, IdempotencyStore, IntegrityStore, MailStore, McpSubscriptionStore, MemberStore,
    MentionInboxStore, MessageStore, MetaStore, NotificationStore, OAuthCodeStore, PeerStore,
    PresenceDigestStore, ProjectorLinkStore, ReferenceStore, ReindexStore, SessionStore,
    ShareTicketStore, SkillStore, SlashCommandStore, SocialStore, TaskScheduleStore,
    ThreadDepStore, ThreadLineageStore, ThreadResultStore, ThreadStore, TokenStore,
    UsageLedgerStore, WebhookStore, WorkspaceStore,
};

/// Everything a store caller usually wants in one import.
///
/// A method invoked on a **concrete** backend (`SqliteStore`/`PostgresStore`)
/// needs the *declaring* sub-trait in scope, so a caller that touches several
/// domains should `use maidan_store::prelude::*` rather than importing each
/// sub-trait by hand. `dyn Store` callers can keep importing just
/// [`Store`](crate::Store) — the super-trait exposes every method.
pub mod prelude {
    pub use crate::store::*;
    pub use crate::{BlobDelete, BlobReap, PostgresStore, SqliteStore, StoreError};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_blob_delete_that_overruns_is_abandoned() {
        let hung: BlobDelete<'static> = Box::new(|| Box::pin(std::future::pending()));
        assert_eq!(run_blob_delete(hung).await, None);
        let failed: BlobDelete<'static> = Box::new(|| Box::pin(async { Err("gone".into()) }));
        assert_eq!(
            run_blob_delete(failed).await,
            Some(BlobReap::DeleteFailed("gone".into()))
        );
        let done: BlobDelete<'static> = Box::new(|| Box::pin(async { Ok(()) }));
        assert_eq!(run_blob_delete(done).await, Some(BlobReap::Deleted));
    }
}
