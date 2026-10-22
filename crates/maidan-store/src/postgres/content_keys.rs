//! Per-message content keys on Postgres: seal on append, shred on withdrawal,
//! rewrap on KEK rotation, and the workspace keys that wrap them. See
//! [`maidan_types::content_seal`].

use chrono::Utc;
use maidan_types::{parse_workspace_kek_id, ContentKeyring, WrappedKey};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::content_keys::{open_joined, JoinedWorkspaceKey, KeyState, Sealing};
use crate::error::StoreError;

/// The key row for `subject`, locked for the rest of the transaction so a
/// concurrent shred cannot interleave with a seal, with the workspace key that
/// wraps it.
pub(crate) async fn state_for_update(
    tx: &mut Transaction<'_, Postgres>,
    subject: Uuid,
) -> Result<(KeyState, JoinedWorkspaceKey), StoreError> {
    let row = sqlx::query(
        "SELECT k.kek_id, k.wrapped_key,
                wk.wrapping_key_id AS wk_kek_id, wk.wrapped_key AS wk_wrapped
         FROM maidan_content_keys k
         LEFT JOIN maidan_workspace_keys wk ON wk.id = k.workspace_key_id
         WHERE k.id = $1
         FOR UPDATE OF k",
    )
    .bind(subject)
    .fetch_optional(&mut **tx)
    .await?;
    let joined = row
        .as_ref()
        .map(|row| JoinedWorkspaceKey {
            kek_id: row.get("wk_kek_id"),
            blob: row.get("wk_wrapped"),
        })
        .unwrap_or_default();
    let state = KeyState::from_columns(row.map(|row| {
        (
            row.get::<Option<String>, _>("kek_id"),
            row.get::<Option<Vec<u8>>, _>("wrapped_key"),
        )
    }));
    Ok((state, joined))
}

/// The workspace's active key, open in `keys`: the one on file, or a new one
/// when the workspace has none yet. Held `FOR SHARE` for the transaction, so a
/// rotation cannot retire it under a data key being wrapped by it.
pub(crate) async fn active_workspace_key(
    tx: &mut Transaction<'_, Postgres>,
    keys: &ContentKeyring,
    workspace_id: Uuid,
) -> Result<Uuid, StoreError> {
    // Two passes at most: a concurrent first seal in the same workspace can
    // win the insert, and then its key is the one to use.
    for _ in 0..2 {
        let row = sqlx::query(
            "SELECT id, wrapping_key_id, wrapped_key FROM maidan_workspace_keys
             WHERE workspace_id = $1 AND retired_at IS NULL
             FOR SHARE",
        )
        .bind(workspace_id)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(row) = row {
            let id: Uuid = row.get("id");
            keys.open_workspace_keys([(
                id,
                WrappedKey {
                    kek_id: row.get("wrapping_key_id"),
                    blob: row.get("wrapped_key"),
                },
            )])
            .await?;
            return Ok(id);
        }
        let id = Uuid::now_v7();
        let wrapped = keys.create_workspace_key(id).await?;
        let inserted = sqlx::query(
            "INSERT INTO maidan_workspace_keys (id, workspace_id, custody, wrapping_key_id, wrapped_key)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (workspace_id) WHERE retired_at IS NULL DO NOTHING",
        )
        .bind(id)
        .bind(workspace_id)
        .bind(keys.custody_name())
        .bind(&wrapped.kek_id)
        .bind(&wrapped.blob)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if inserted == 1 {
            return Ok(id);
        }
        keys.forget_workspace_key(id);
    }
    Err(StoreError::Conflict(
        "the workspace's key changed while it was being read; retry".into(),
    ))
}

/// Retire the workspace's active key and make a new one. The rewrap pass then
/// moves the workspace's data keys to the new key and destroys the old one;
/// no sealed message is touched. Returns the retired key (if there was one)
/// and the new key.
pub async fn rotate_workspace_key(
    pool: &PgPool,
    keys: &ContentKeyring,
    workspace_id: Uuid,
) -> Result<(Option<Uuid>, Uuid), StoreError> {
    let mut tx = pool.begin().await?;
    let retired: Option<Uuid> = sqlx::query_scalar(
        "UPDATE maidan_workspace_keys SET retired_at = $2
         WHERE workspace_id = $1 AND retired_at IS NULL
         RETURNING id",
    )
    .bind(workspace_id)
    .bind(Utc::now())
    .fetch_optional(&mut *tx)
    .await?;
    let active = active_workspace_key(&mut tx, keys, workspace_id).await?;
    tx.commit().await?;
    Ok((retired, active))
}

pub(crate) async fn insert_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    subject: Uuid,
    workspace_id: Uuid,
    wrapped: Option<&WrappedKey>,
) -> Result<(), StoreError> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_content_keys
             (id, workspace_id, kek_id, wrapped_key, workspace_key_id, created_at, shredded_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(subject)
    .bind(workspace_id)
    .bind(wrapped.map(|w| w.kek_id.as_str()))
    .bind(wrapped.map(|w| w.blob.as_slice()))
    .bind(wrapped.and_then(|w| parse_workspace_kek_id(&w.kek_id)))
    .bind(now)
    .bind(wrapped.is_none().then_some(now))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Destroy `subject`'s key, and every queued copy of its events' words that
/// would otherwise outlive it: webhook deliveries and projector egress built
/// from those events, and notification mail about the message. Idempotent.
pub(crate) async fn shred_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    subject: Uuid,
) -> Result<bool, StoreError> {
    let shredded = sqlx::query(
        "UPDATE maidan_content_keys SET kek_id = NULL, wrapped_key = NULL, shredded_at = $2
         WHERE id = $1 AND shredded_at IS NULL",
    )
    .bind(subject)
    .bind(Utc::now())
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;
    sqlx::query(
        "DELETE FROM maidan_webhook_deliveries
         WHERE log_id IN (SELECT id FROM maidan_events WHERE content_key_id = $1)",
    )
    .bind(subject)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM maidan_egress_outbox
         WHERE source_log_id IN (SELECT id FROM maidan_events WHERE content_key_id = $1)",
    )
    .bind(subject)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM maidan_mail_outbox WHERE content_key_id = $1")
        .bind(subject)
        .execute(&mut **tx)
        .await?;
    Ok(shredded)
}

/// Seal or shred for a message-content event inside its append transaction.
pub(crate) async fn prepare_in_tx(
    tx: &mut Transaction<'_, Postgres>,
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
            let (state, joined) = state_for_update(tx, subject).await?;
            let workspace_key = match (&state, arrived_shredded) {
                (KeyState::Live(wrapped), _) => {
                    open_joined(keys, &wrapped.kek_id, joined).await?;
                    None
                }
                (KeyState::Absent, false) => {
                    Some(active_workspace_key(tx, keys, workspace_id).await?)
                }
                _ => None,
            };
            let decision =
                crate::content_keys::decide(keys, workspace_key, subject, state, arrived_shredded)?;
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

/// One rewrap batch: up to `limit` rows of each kind below. Returns how many
/// rows changed; `0` means every key is where it belongs.
///
/// 1. A workspace key still wrapped by a previous instance KEK is rewrapped
///    under the primary (an instance KEK rotation).
/// 2. A live data key under no workspace key (written before 0167), or under
///    a retired one (a workspace key rotation), is rewrapped under its
///    workspace's active key.
/// 3. A retired workspace key no live data key needs is destroyed.
pub async fn rewrap(pool: &PgPool, keys: &ContentKeyring, limit: i64) -> Result<u64, StoreError> {
    let mut changed = 0u64;

    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT id, wrapping_key_id, wrapped_key FROM maidan_workspace_keys
         WHERE destroyed_at IS NULL AND wrapping_key_id <> $1
         ORDER BY id
         LIMIT $2
         FOR UPDATE SKIP LOCKED",
    )
    .bind(keys.workspace_wrapping_id())
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    for row in &rows {
        let id: Uuid = row.get("id");
        let rewrapped = keys
            .rewrap_workspace_key(
                id,
                &WrappedKey {
                    kek_id: row.get("wrapping_key_id"),
                    blob: row.get("wrapped_key"),
                },
            )
            .await?;
        sqlx::query(
            "UPDATE maidan_workspace_keys SET wrapping_key_id = $2, wrapped_key = $3, custody = $4
             WHERE id = $1",
        )
        .bind(id)
        .bind(&rewrapped.kek_id)
        .bind(&rewrapped.blob)
        .bind(keys.custody_name())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    changed += rows.len() as u64;

    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT k.id, k.workspace_id, k.kek_id, k.wrapped_key,
                wk.wrapping_key_id AS wk_kek_id, wk.wrapped_key AS wk_wrapped
         FROM maidan_content_keys k
         LEFT JOIN maidan_workspace_keys wk ON wk.id = k.workspace_key_id
         WHERE k.shredded_at IS NULL
           AND (k.workspace_key_id IS NULL OR wk.retired_at IS NOT NULL)
         ORDER BY k.id
         LIMIT $1
         FOR UPDATE OF k SKIP LOCKED",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    for row in &rows {
        let subject: Uuid = row.get("id");
        let wrapped = WrappedKey {
            kek_id: row.get("kek_id"),
            blob: row.get("wrapped_key"),
        };
        open_joined(
            keys,
            &wrapped.kek_id,
            JoinedWorkspaceKey {
                kek_id: row.get("wk_kek_id"),
                blob: row.get("wk_wrapped"),
            },
        )
        .await?;
        let data_key = keys.unwrap(subject, &wrapped)?;
        let active = active_workspace_key(&mut tx, keys, row.get("workspace_id")).await?;
        let rewrapped = keys.wrap_for_workspace(active, subject, &data_key)?;
        sqlx::query(
            "UPDATE maidan_content_keys SET kek_id = $2, wrapped_key = $3, workspace_key_id = $4
             WHERE id = $1",
        )
        .bind(subject)
        .bind(&rewrapped.kek_id)
        .bind(&rewrapped.blob)
        .bind(active)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    changed += rows.len() as u64;

    let destroyed: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE maidan_workspace_keys wk
         SET wrapping_key_id = NULL, wrapped_key = NULL, destroyed_at = $1
         WHERE wk.id IN (
             SELECT r.id FROM maidan_workspace_keys r
             WHERE r.retired_at IS NOT NULL AND r.destroyed_at IS NULL
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_content_keys k
                   WHERE k.workspace_key_id = r.id AND k.shredded_at IS NULL
               )
             ORDER BY r.id
             LIMIT $2
             FOR UPDATE SKIP LOCKED
         )
         RETURNING wk.id",
    )
    .bind(Utc::now())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    for id in &destroyed {
        keys.forget_workspace_key(*id);
    }
    changed += destroyed.len() as u64;
    Ok(changed)
}

/// Rows the rewrap pass has still to change (see [`rewrap`]).
pub async fn count_needing_rewrap(pool: &PgPool, keys: &ContentKeyring) -> Result<u64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT
            (SELECT COUNT(*) FROM maidan_workspace_keys
             WHERE destroyed_at IS NULL AND wrapping_key_id <> $1)
          + (SELECT COUNT(*) FROM maidan_content_keys k
             LEFT JOIN maidan_workspace_keys wk ON wk.id = k.workspace_key_id
             WHERE k.shredded_at IS NULL
               AND (k.workspace_key_id IS NULL OR wk.retired_at IS NOT NULL))
          + (SELECT COUNT(*) FROM maidan_workspace_keys r
             WHERE r.retired_at IS NOT NULL AND r.destroyed_at IS NULL
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_content_keys k
                   WHERE k.workspace_key_id = r.id AND k.shredded_at IS NULL
               ))",
    )
    .bind(keys.workspace_wrapping_id())
    .fetch_one(pool)
    .await?;
    Ok(count as u64)
}
