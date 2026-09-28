//! Dialect-neutral outbox access for relay and metrics.

use std::sync::Arc;

use maidan_types::{ContentKeyring, WorkspaceId};
use sqlx::{PgPool, SqlitePool};

use crate::error::StoreError;
use crate::postgres::outbox::{OutboxRow, QuarantinedOutboxRow};

/// The outbox of one store, with the keyring that opens the rows' sealed
/// message words.
#[derive(Clone)]
pub struct OutboxBackend {
    db: Db,
    keys: Arc<ContentKeyring>,
}

#[derive(Clone)]
enum Db {
    Postgres(PgPool),
    Sqlite(SqlitePool),
}

impl OutboxBackend {
    /// A Postgres outbox that opens sealed words with `keys`, which must be
    /// the store's keyring.
    pub fn postgres(pool: PgPool, keys: Arc<ContentKeyring>) -> Self {
        Self {
            db: Db::Postgres(pool),
            keys,
        }
    }

    /// The SQLite twin of [`Self::postgres`].
    pub fn sqlite(pool: SqlitePool, keys: Arc<ContentKeyring>) -> Self {
        Self {
            db: Db::Sqlite(pool),
            keys,
        }
    }

    /// Atomically claim relayable rows for this relay. Use this, not
    /// [`Self::list_pending`], from the relay loop: the relay runs in every
    /// replica, so an unlocked read relays every row once per replica.
    pub async fn claim_pending(
        &self,
        limit: i64,
        lease_secs: i64,
    ) -> Result<Vec<OutboxRow>, StoreError> {
        match &self.db {
            Db::Postgres(pool) => {
                crate::postgres::outbox::claim_pending(pool, &self.keys, limit, lease_secs).await
            }
            Db::Sqlite(pool) => {
                crate::sqlite::outbox::claim_pending(pool, &self.keys, limit, lease_secs).await
            }
        }
    }

    /// Unlocked read of relayable rows — metrics and tests only.
    pub async fn list_pending(&self, limit: i64) -> Result<Vec<OutboxRow>, StoreError> {
        match &self.db {
            Db::Postgres(pool) => {
                crate::postgres::outbox::list_pending(pool, &self.keys, limit).await
            }
            Db::Sqlite(pool) => crate::sqlite::outbox::list_pending(pool, &self.keys, limit).await,
        }
    }

    pub async fn mark_published(&self, outbox_id: i64) -> Result<(), StoreError> {
        match &self.db {
            Db::Postgres(pool) => crate::postgres::outbox::mark_published(pool, outbox_id).await,
            Db::Sqlite(pool) => crate::sqlite::outbox::mark_published(pool, outbox_id).await,
        }
    }

    pub async fn mark_published_batch(&self, outbox_ids: &[i64]) -> Result<(), StoreError> {
        match &self.db {
            Db::Postgres(pool) => {
                crate::postgres::outbox::mark_published_batch(pool, outbox_ids).await
            }
            Db::Sqlite(pool) => crate::sqlite::outbox::mark_published_batch(pool, outbox_ids).await,
        }
    }

    pub async fn record_attempt(&self, outbox_id: i64) -> Result<i32, StoreError> {
        match &self.db {
            Db::Postgres(pool) => crate::postgres::outbox::record_attempt(pool, outbox_id).await,
            Db::Sqlite(pool) => crate::sqlite::outbox::record_attempt(pool, outbox_id).await,
        }
    }

    pub async fn quarantine(&self, outbox_id: i64) -> Result<(), StoreError> {
        match &self.db {
            Db::Postgres(pool) => crate::postgres::outbox::quarantine(pool, outbox_id).await,
            Db::Sqlite(pool) => crate::sqlite::outbox::quarantine(pool, outbox_id).await,
        }
    }

    pub async fn replay_quarantined(
        &self,
        outbox_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<(), StoreError> {
        match &self.db {
            Db::Postgres(pool) => {
                crate::postgres::outbox::replay_quarantined(pool, outbox_id, workspace_id).await
            }
            Db::Sqlite(pool) => {
                crate::sqlite::outbox::replay_quarantined(pool, outbox_id, workspace_id).await
            }
        }
    }

    pub async fn count_pending(&self) -> Result<i64, StoreError> {
        match &self.db {
            Db::Postgres(pool) => crate::postgres::outbox::count_pending(pool).await,
            Db::Sqlite(pool) => crate::sqlite::outbox::count_pending(pool).await,
        }
    }

    pub async fn list_quarantined_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<QuarantinedOutboxRow>, StoreError> {
        match &self.db {
            Db::Postgres(pool) => {
                crate::postgres::outbox::list_quarantined_for_workspace(pool, workspace_id, limit)
                    .await
            }
            Db::Sqlite(pool) => {
                crate::sqlite::outbox::list_quarantined_for_workspace(pool, workspace_id, limit)
                    .await
            }
        }
    }

    pub async fn count_quarantined(&self) -> Result<i64, StoreError> {
        match &self.db {
            Db::Postgres(pool) => crate::postgres::outbox::count_quarantined(pool).await,
            Db::Sqlite(pool) => crate::sqlite::outbox::count_quarantined(pool).await,
        }
    }

    pub async fn oldest_relayable_pending_age_secs(&self) -> Result<Option<f64>, StoreError> {
        match &self.db {
            Db::Postgres(pool) => {
                crate::postgres::outbox::oldest_relayable_pending_age_secs(pool).await
            }
            Db::Sqlite(pool) => {
                crate::sqlite::outbox::oldest_relayable_pending_age_secs(pool).await
            }
        }
    }

    pub async fn get_stored_event(
        &self,
        log_id: i64,
    ) -> Result<maidan_types::StoredEvent, StoreError> {
        match &self.db {
            Db::Postgres(pool) => {
                crate::postgres::events::get_by_id(pool, &self.keys, log_id).await
            }
            Db::Sqlite(pool) => crate::sqlite::events::get_by_id(pool, &self.keys, log_id).await,
        }
    }
}
