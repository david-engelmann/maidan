//! Per-message content keys on SQLite; see the Postgres twin.

use chrono::Utc;
use maidan_types::{ContentKeyring, WrappedKey};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::content_keys::{KeyState, Sealing};
use crate::error::StoreError;

/// The key row for `subject`. SQLite serializes writers, so the append's
/// transaction already excludes a concurrent shred.
pub(crate) async fn state_for_update(
    tx: &mut Transaction<'_, Sqlite>,
    subject: Uuid,
) -> Result<KeyState, StoreError> {
    let row = sqlx::query("SELECT kek_id, wrapped_key FROM maidan_content_keys WHERE id = ?")
        .bind(subject)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(KeyState::from_columns(row.map(|row| {
        (
            row.get::<Option<String>, _>("kek_id"),
            row.get::<Option<Vec<u8>>, _>("wrapped_key"),
        )
    })))
}

pub(crate) async fn insert_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    subject: Uuid,
    workspace_id: Uuid,
    wrapped: Option<&WrappedKey>,
) -> Result<(), StoreError> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_content_keys (id, workspace_id, kek_id, wrapped_key, created_at, shredded_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(subject)
    .bind(workspace_id)
    .bind(wrapped.map(|w| w.kek_id.as_str()))
    .bind(wrapped.map(|w| w.blob.as_slice()))
    .bind(now)
    .bind(wrapped.is_none().then_some(now))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Destroy `subject`'s key, and every queued copy of its events' words that
/// would otherwise outlive it: webhook deliveries and projector egress built
/// from those events. Idempotent.
pub(crate) async fn shred_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    subject: Uuid,
) -> Result<bool, StoreError> {
    let shredded = sqlx::query(
        "UPDATE maidan_content_keys SET kek_id = NULL, wrapped_key = NULL, shredded_at = ?
         WHERE id = ? AND shredded_at IS NULL",
    )
    .bind(Utc::now())
    .bind(subject)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;
    sqlx::query(
        "DELETE FROM maidan_webhook_deliveries
         WHERE log_id IN (SELECT id FROM maidan_events WHERE content_key_id = ?)",
    )
    .bind(subject)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM maidan_egress_outbox
         WHERE source_log_id IN (SELECT id FROM maidan_events WHERE content_key_id = ?)",
    )
    .bind(subject)
    .execute(&mut **tx)
    .await?;
    Ok(shredded)
}

/// Seal or shred for a message-content event inside its append transaction.
pub(crate) async fn prepare_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    keys: Option<&ContentKeyring>,
    plan: crate::content_keys::Plan,
    payload: &mut serde_json::Value,
) -> Result<Sealing, StoreError> {
    use crate::content_keys::Plan;
    match plan {
        Plan::None => Ok(Sealing::default()),
        Plan::Shred { subject } => {
            shred_in_tx(tx, subject).await?;
            Ok(Sealing::default())
        }
        Plan::Seal {
            subject,
            workspace_id,
            arrived_shredded,
        } => {
            let keys = crate::content_keys::require(keys)?;
            let state = state_for_update(tx, subject).await?;
            let decision = crate::content_keys::decide(keys, subject, state, arrived_shredded)?;
            match &decision.write {
                crate::content_keys::Write::Nothing => {}
                crate::content_keys::Write::Insert(wrapped) => {
                    insert_in_tx(tx, subject, workspace_id, wrapped.as_ref()).await?
                }
                crate::content_keys::Write::Shred => {
                    shred_in_tx(tx, subject).await?;
                }
            }
            decision.apply(subject, payload)
        }
    }
}

/// Rewrap up to `limit` live keys that are not under the primary KEK. Returns
/// how many were rewrapped; `0` means rotation is complete.
pub async fn rewrap(
    pool: &SqlitePool,
    keys: &ContentKeyring,
    limit: i64,
) -> Result<u64, StoreError> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT id, kek_id, wrapped_key FROM maidan_content_keys
         WHERE shredded_at IS NULL AND kek_id <> ?
         ORDER BY id
         LIMIT ?",
    )
    .bind(keys.primary_id())
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    for row in &rows {
        let subject: Uuid = row.get("id");
        let wrapped = WrappedKey {
            kek_id: row.get("kek_id"),
            blob: row.get("wrapped_key"),
        };
        let rewrapped = keys.wrap(subject, &keys.unwrap(subject, &wrapped)?)?;
        sqlx::query("UPDATE maidan_content_keys SET kek_id = ?, wrapped_key = ? WHERE id = ?")
            .bind(&rewrapped.kek_id)
            .bind(&rewrapped.blob)
            .bind(subject)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(rows.len() as u64)
}

/// Live keys still wrapped by a KEK other than the primary.
pub async fn count_needing_rewrap(
    pool: &SqlitePool,
    keys: &ContentKeyring,
) -> Result<u64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maidan_content_keys WHERE shredded_at IS NULL AND kek_id <> ?",
    )
    .bind(keys.primary_id())
    .fetch_one(pool)
    .await?;
    Ok(count as u64)
}
