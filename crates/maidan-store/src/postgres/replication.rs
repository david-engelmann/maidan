//! Read-replica LSN helpers (Cluster 261, Program D). Called directly by the bus /
//! router (like `events::get_by_id`), not via the `Store` trait — they are a
//! Postgres-streaming-replication concern with no SQLite analogue.

use maidan_types::Lsn;
use sqlx::{PgPool, Row};

use crate::error::StoreError;

fn parse_lsn(s: &str) -> Result<Lsn, StoreError> {
    Lsn::from_pg_str(s).ok_or_else(|| StoreError::InvalidInput(format!("unparseable pg_lsn: {s}")))
}

/// The primary's current WAL write position — the causality token stamped on a
/// write (`pg_current_wal_lsn()`). Errors if called on a standby (a standby is in
/// recovery and has no *write* position); only call it on the writer pool.
pub async fn current_wal_lsn(pool: &PgPool) -> Result<Lsn, StoreError> {
    let row = sqlx::query("SELECT pg_current_wal_lsn()::text AS lsn")
        .fetch_one(pool)
        .await?;
    parse_lsn(&row.get::<String, _>("lsn"))
}

/// A replica's last replayed WAL position (`pg_last_wal_replay_lsn()`), or `None`
/// when the connection is **not** a standby (a primary is not in recovery, so the
/// function returns NULL). A read can be served from this pool once its value has
/// reached the read's causality token.
pub async fn replica_replay_lsn(pool: &PgPool) -> Result<Option<Lsn>, StoreError> {
    let row = sqlx::query("SELECT pg_last_wal_replay_lsn()::text AS lsn")
        .fetch_one(pool)
        .await?;
    match row.get::<Option<String>, _>("lsn") {
        Some(s) => Ok(Some(parse_lsn(&s)?)),
        None => Ok(None),
    }
}

/// Whether `pool` (a replica) has replayed far enough to serve a read carrying
/// `token`. `false` when the pool is not a standby (no replay position) or is still
/// behind the token — either way the caller should fall back to the primary.
pub async fn replica_caught_up(pool: &PgPool, token: Lsn) -> Result<bool, StoreError> {
    Ok(replica_replay_lsn(pool)
        .await?
        .is_some_and(|replayed| replayed >= token))
}
