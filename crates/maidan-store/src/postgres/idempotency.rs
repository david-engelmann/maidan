//! Postgres-backed idempotency keys. See `crate::idempotency`.

use chrono::Utc;
use maidan_types::{MemberId, WorkspaceId};
use sqlx::PgPool;

use crate::error::StoreError;
use crate::idempotency::{
    reservation_from, IdempotencyReservation, KeyRow, NewIdempotencyKey, StoredResponse,
    PRUNE_BATCH,
};

pub async fn reserve(
    pool: &PgPool,
    new: &NewIdempotencyKey,
) -> Result<IdempotencyReservation, StoreError> {
    let now = Utc::now();
    sqlx::query(
        "DELETE FROM maidan_idempotency_keys WHERE ctid IN (
             SELECT ctid FROM maidan_idempotency_keys WHERE expires_at <= $1 LIMIT $2)",
    )
    .bind(now)
    .bind(PRUNE_BATCH)
    .execute(pool)
    .await?;
    // Insert, or take over a lapsed reservation that never completed; a
    // completed row or a live reservation is left alone.
    let taken: Option<(i32,)> = sqlx::query_as(
        "INSERT INTO maidan_idempotency_keys
             (workspace_id, actor_id, idempotency_key, fingerprint, locked_until, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (workspace_id, actor_id, idempotency_key) DO UPDATE
             SET fingerprint = EXCLUDED.fingerprint,
                 locked_until = EXCLUDED.locked_until,
                 expires_at = EXCLUDED.expires_at
             WHERE maidan_idempotency_keys.status IS NULL
               AND maidan_idempotency_keys.locked_until <= $7
         RETURNING 1",
    )
    .bind(new.workspace_id.0)
    .bind(new.actor_id.0)
    .bind(&new.key)
    .bind(&new.fingerprint)
    .bind(new.locked_until)
    .bind(new.expires_at)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    if taken.is_some() {
        return Ok(IdempotencyReservation::Reserved);
    }
    let row: KeyRow = sqlx::query_as(
        "SELECT fingerprint, status, content_type, body FROM maidan_idempotency_keys
          WHERE workspace_id = $1 AND actor_id = $2 AND idempotency_key = $3",
    )
    .bind(new.workspace_id.0)
    .bind(new.actor_id.0)
    .bind(&new.key)
    .fetch_one(pool)
    .await?;
    Ok(reservation_from(row))
}

pub async fn complete(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    actor_id: MemberId,
    key: &str,
    response: &StoredResponse,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_idempotency_keys SET status = $4, content_type = $5, body = $6
          WHERE workspace_id = $1 AND actor_id = $2 AND idempotency_key = $3
            AND status IS NULL",
    )
    .bind(workspace_id.0)
    .bind(actor_id.0)
    .bind(key)
    .bind(i32::from(response.status))
    .bind(response.content_type.as_deref())
    .bind(&response.body)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn release(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    actor_id: MemberId,
    key: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "DELETE FROM maidan_idempotency_keys
          WHERE workspace_id = $1 AND actor_id = $2 AND idempotency_key = $3
            AND status IS NULL",
    )
    .bind(workspace_id.0)
    .bind(actor_id.0)
    .bind(key)
    .execute(pool)
    .await?;
    Ok(())
}
