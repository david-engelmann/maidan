//! SQLite-backed idempotency keys. See `crate::idempotency`. Times are
//! millisecond `...Z` text, so string comparison is time order.

use chrono::{DateTime, SecondsFormat, Utc};
use maidan_types::{MemberId, WorkspaceId};
use sqlx::SqlitePool;

use crate::error::StoreError;
use crate::idempotency::{
    reservation_from, IdempotencyReservation, KeyRow, NewIdempotencyKey, StoredResponse,
    PRUNE_BATCH,
};

fn ms(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub async fn reserve(
    pool: &SqlitePool,
    new: &NewIdempotencyKey,
) -> Result<IdempotencyReservation, StoreError> {
    let now = ms(Utc::now());
    // The requested key first: once expired it must not replay, however
    // many other expired rows the bounded prune below leaves.
    sqlx::query(
        "DELETE FROM maidan_idempotency_keys
          WHERE workspace_id = ?1 AND actor_id = ?2 AND idempotency_key = ?3
            AND expires_at <= ?4",
    )
    .bind(new.workspace_id.0)
    .bind(new.actor_id.0)
    .bind(&new.key)
    .bind(&now)
    .execute(pool)
    .await?;
    let lease = uuid::Uuid::now_v7().to_string();
    sqlx::query(
        "DELETE FROM maidan_idempotency_keys WHERE rowid IN (
             SELECT rowid FROM maidan_idempotency_keys WHERE expires_at <= ?1 LIMIT ?2)",
    )
    .bind(&now)
    .bind(PRUNE_BATCH)
    .execute(pool)
    .await?;
    let taken: Option<(i32,)> = sqlx::query_as(
        "INSERT INTO maidan_idempotency_keys
             (workspace_id, actor_id, idempotency_key, fingerprint, locked_until, expires_at, lease)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?8)
         ON CONFLICT (workspace_id, actor_id, idempotency_key) DO UPDATE
             SET lease = excluded.lease,
                 locked_until = excluded.locked_until,
                 expires_at = excluded.expires_at
             WHERE maidan_idempotency_keys.status IS NULL
               AND maidan_idempotency_keys.locked_until <= ?7
               AND maidan_idempotency_keys.fingerprint = excluded.fingerprint
         RETURNING 1",
    )
    .bind(new.workspace_id.0)
    .bind(new.actor_id.0)
    .bind(&new.key)
    .bind(&new.fingerprint)
    .bind(ms(new.locked_until))
    .bind(ms(new.expires_at))
    .bind(&now)
    .bind(&lease)
    .fetch_optional(pool)
    .await?;
    if taken.is_some() {
        return Ok(IdempotencyReservation::Reserved { lease });
    }
    let row: KeyRow = sqlx::query_as(
        "SELECT fingerprint, status, content_type, body FROM maidan_idempotency_keys
          WHERE workspace_id = ?1 AND actor_id = ?2 AND idempotency_key = ?3",
    )
    .bind(new.workspace_id.0)
    .bind(new.actor_id.0)
    .bind(&new.key)
    .fetch_one(pool)
    .await?;
    Ok(reservation_from(row))
}

pub async fn complete(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    actor_id: MemberId,
    key: &str,
    lease: &str,
    response: &StoredResponse,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_idempotency_keys SET status = ?4, content_type = ?5, body = ?6
          WHERE workspace_id = ?1 AND actor_id = ?2 AND idempotency_key = ?3
            AND status IS NULL AND lease = ?7",
    )
    .bind(workspace_id.0)
    .bind(actor_id.0)
    .bind(key)
    .bind(i32::from(response.status))
    .bind(response.content_type.as_deref())
    .bind(&response.body)
    .bind(lease)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn release(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    actor_id: MemberId,
    key: &str,
    lease: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "DELETE FROM maidan_idempotency_keys
          WHERE workspace_id = ?1 AND actor_id = ?2 AND idempotency_key = ?3
            AND status IS NULL AND lease = ?4",
    )
    .bind(workspace_id.0)
    .bind(actor_id.0)
    .bind(key)
    .bind(lease)
    .execute(pool)
    .await?;
    Ok(())
}
