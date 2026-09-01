//! Storage abstraction for Maidan.
//!
//! Defines [`Store`], a backend-agnostic async interface, plus Postgres
//! and SQLite implementations backed by `sqlx`.

pub mod a2a;
pub mod automation_deliveries;
pub mod dialect;
pub mod dm;
pub mod embeddings_purge;
pub mod error;
pub mod group_dm;
pub mod migrate;
pub mod outbox;
pub mod postgres;
pub mod sqlite;
pub mod store;

pub use automation_deliveries::AutomationDeliveryFilter;
pub use dialect::Dialect;
pub use error::StoreError;
pub use migrate::{run_postgres_migrations, run_sqlite_migrations};
pub use outbox::OutboxBackend;
pub use postgres::outbox::{OutboxRow, QuarantinedOutboxRow};
pub use postgres::PostgresStore;
pub use sqlite::SqliteStore;

/// Default max connections for a **file-backed SQLite** pool (Cluster 277).
///
/// SQLite allows only one writer at a time, and sqlx's `pool.begin()` opens a
/// *deferred* transaction: with more than one pooled connection, two writers can
/// each take a read snapshot and then race to upgrade to the writer, which is a
/// genuine deadlock that `busy_timeout` cannot resolve (it returns `SQLITE_BUSY`
/// immediately rather than waiting). A contention test showed a warm 8-connection
/// pool failing ~90% of read-modify-write transactions with "database is locked",
/// while a single connection is clean. So the SQLite backend serializes through one
/// connection by default (overridable via `MAIDAN_DB_MAX_CONNECTIONS` for anyone who
/// has arranged writes to avoid the upgrade deadlock). Postgres, the production/HA
/// backend, is unaffected and keeps its multi-connection pool.
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
// The domain sub-traits `Store` composes (Cluster 349). Re-exported so a caller
// that needs only one concern can bound on the narrower trait; `dyn Store` still
// exposes them all via the super-trait.
pub use store::{
    A2aStore, AppStore, ArtifactMetaStore, AssignmentStore, AutomationStore, ChannelStore,
    DeliveryCursorStore, DmStore, EventStore, FollowStore, FsmHookStore, GlossaryStore, MailStore,
    MemberStore, MentionInboxStore, MessageStore, MetaStore, NotificationStore, OAuthCodeStore,
    PeerStore, PresenceDigestStore, ProjectorLinkStore, ReferenceStore, ReindexStore, SessionStore,
    SkillStore, SlashCommandStore, SocialStore, TaskScheduleStore, ThreadDepStore,
    ThreadResultStore, ThreadStore, TokenStore, WebhookStore, WorkspaceStore,
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
