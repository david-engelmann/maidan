//! What a Postgres operator watches, read from the catalogs for `/metrics`
//! ([`crate::store::MetaStore::database_health`]). Postgres only: SQLite has
//! no transaction ids to wrap, no vacuum statistics and no replication slots.

use sqlx::{PgPool, Row};

use crate::error::StoreError;
use crate::store::DatabaseHealth;

/// One read of the database's health. The slots are read separately and may
/// be `None`: a managed database can refuse `pg_replication_slots` to the
/// application's role, and that must not hide the rest.
pub(crate) async fn read(pool: &PgPool) -> Result<DatabaseHealth, StoreError> {
    let row = sqlx::query(
        "SELECT
             (SELECT age(datfrozenxid)::bigint FROM pg_database
              WHERE datname = current_database()) AS xid_age,
             (SELECT COALESCE(SUM(n_dead_tup), 0)::bigint FROM pg_stat_user_tables) AS dead,
             (SELECT COALESCE(SUM(n_live_tup), 0)::bigint FROM pg_stat_user_tables) AS live,
             pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0')::bigint AS wal_bytes",
    )
    .fetch_one(pool)
    .await?;
    let slots = sqlx::query(
        "SELECT COALESCE(MAX(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)), 0)::bigint
                    AS retained,
                COUNT(*) FILTER (WHERE NOT active)::bigint AS inactive
         FROM pg_replication_slots",
    )
    .fetch_one(pool)
    .await;
    let (slot_retained_bytes, inactive_slots) = match slots {
        Ok(slots) => (
            Some(slots.try_get("retained")?),
            Some(slots.try_get("inactive")?),
        ),
        Err(err) => {
            tracing::debug!(error = %err, "pg_replication_slots unreadable; slot gauges skipped");
            (None, None)
        }
    };
    Ok(DatabaseHealth {
        xid_age: row.try_get("xid_age")?,
        dead_tuples: row.try_get("dead")?,
        live_tuples: row.try_get("live")?,
        wal_bytes: row.try_get("wal_bytes")?,
        slot_retained_bytes,
        inactive_slots,
    })
}
