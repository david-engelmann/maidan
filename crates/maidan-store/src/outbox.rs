//! Dialect-neutral outbox access for relay and metrics.

use maidan_types::WorkspaceId;
use sqlx::{PgPool, SqlitePool};

use crate::error::StoreError;
use crate::postgres::outbox::{OutboxRow, QuarantinedOutboxRow};

#[derive(Clone)]
pub enum OutboxBackend {
    Postgres(PgPool),
    Sqlite(SqlitePool),
}

impl OutboxBackend {
    pub async fn list_pending(&self, limit: i64) -> Result<Vec<OutboxRow>, StoreError> {
        match self {
            Self::Postgres(pool) => crate::postgres::outbox::list_pending(pool, limit).await,
            Self::Sqlite(pool) => crate::sqlite::outbox::list_pending(pool, limit).await,
        }
    }

    pub async fn mark_published(&self, outbox_id: i64) -> Result<(), StoreError> {
        match self {
            Self::Postgres(pool) => crate::postgres::outbox::mark_published(pool, outbox_id).await,
            Self::Sqlite(pool) => crate::sqlite::outbox::mark_published(pool, outbox_id).await,
        }
    }

    pub async fn record_attempt(&self, outbox_id: i64) -> Result<i32, StoreError> {
        match self {
            Self::Postgres(pool) => crate::postgres::outbox::record_attempt(pool, outbox_id).await,
            Self::Sqlite(pool) => crate::sqlite::outbox::record_attempt(pool, outbox_id).await,
        }
    }

    pub async fn quarantine(&self, outbox_id: i64) -> Result<(), StoreError> {
        match self {
            Self::Postgres(pool) => crate::postgres::outbox::quarantine(pool, outbox_id).await,
            Self::Sqlite(pool) => crate::sqlite::outbox::quarantine(pool, outbox_id).await,
        }
    }

    pub async fn replay_quarantined(
        &self,
        outbox_id: i64,
        workspace_id: WorkspaceId,
    ) -> Result<(), StoreError> {
        match self {
            Self::Postgres(pool) => {
                crate::postgres::outbox::replay_quarantined(pool, outbox_id, workspace_id).await
            }
            Self::Sqlite(pool) => {
                crate::sqlite::outbox::replay_quarantined(pool, outbox_id, workspace_id).await
            }
        }
    }

    pub async fn count_pending(&self) -> Result<i64, StoreError> {
        match self {
            Self::Postgres(pool) => crate::postgres::outbox::count_pending(pool).await,
            Self::Sqlite(pool) => crate::sqlite::outbox::count_pending(pool).await,
        }
    }

    pub async fn list_quarantined_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> Result<Vec<QuarantinedOutboxRow>, StoreError> {
        match self {
            Self::Postgres(pool) => {
                crate::postgres::outbox::list_quarantined_for_workspace(pool, workspace_id, limit)
                    .await
            }
            Self::Sqlite(pool) => {
                crate::sqlite::outbox::list_quarantined_for_workspace(pool, workspace_id, limit)
                    .await
            }
        }
    }

    pub async fn count_quarantined(&self) -> Result<i64, StoreError> {
        match self {
            Self::Postgres(pool) => crate::postgres::outbox::count_quarantined(pool).await,
            Self::Sqlite(pool) => crate::sqlite::outbox::count_quarantined(pool).await,
        }
    }

    pub async fn oldest_relayable_pending_age_secs(&self) -> Result<Option<f64>, StoreError> {
        match self {
            Self::Postgres(pool) => {
                crate::postgres::outbox::oldest_relayable_pending_age_secs(pool).await
            }
            Self::Sqlite(pool) => {
                crate::sqlite::outbox::oldest_relayable_pending_age_secs(pool).await
            }
        }
    }

    pub async fn get_stored_event(
        &self,
        log_id: i64,
    ) -> Result<maidan_types::StoredEvent, StoreError> {
        match self {
            Self::Postgres(pool) => crate::postgres::events::get_by_id(pool, log_id).await,
            Self::Sqlite(pool) => crate::sqlite::events::get_by_id(pool, log_id).await,
        }
    }
}
