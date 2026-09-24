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
/// Why a review-requirement write was refused: it would lower the requirement
/// and the caller may not. Checked in the write's transaction.
pub const REVIEW_LOWER_REFUSAL: &str =
    "lowering a review requirement needs the channel:admin capability";
pub mod automation_deliveries;
mod delegation_grants;
pub mod dialect;
pub mod dm;
pub mod embeddings_purge;
pub mod error;
pub mod group_dm;
pub mod lag_resume;
pub mod log_snapshot;
pub mod migrate;
pub mod outbox;
pub mod postgres;
pub mod result_delivery;
mod share_tickets;
pub mod sqlite;
pub mod store;
pub mod workspace_export;

pub use automation_deliveries::AutomationDeliveryFilter;
pub use dialect::Dialect;
pub use error::StoreError;
pub use lag_resume::{resume_from_log, LAG_RESUME_BATCH};
pub use log_snapshot::{build_log_snapshot, catch_up_since, CATCH_UP_LIMIT};
pub use migrate::{run_postgres_migrations, run_sqlite_migrations};
pub use outbox::OutboxBackend;
pub use postgres::outbox::{OutboxRow, QuarantinedOutboxRow};
pub use postgres::PostgresStore;
pub use result_delivery::{replay_result_delivery, ResultDeliveryReplay};
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
    GlossaryStore, IntegrityStore, MailStore, MemberStore, MentionInboxStore, MessageStore,
    MetaStore, NotificationStore, OAuthCodeStore, PeerStore, PresenceDigestStore,
    ProjectorLinkStore, ReferenceStore, ReindexStore, SessionStore, ShareTicketStore, SkillStore,
    SlashCommandStore, SocialStore, TaskScheduleStore, ThreadDepStore, ThreadLineageStore,
    ThreadResultStore, ThreadStore, TokenStore, UsageLedgerStore, WebhookStore, WorkspaceStore,
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
    pub use crate::{PostgresStore, SqliteStore, StoreError};
}
