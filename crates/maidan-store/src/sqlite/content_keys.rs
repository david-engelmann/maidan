//! Per-message content keys on SQLite; see the Postgres twin.

use chrono::Utc;
use maidan_types::{parse_workspace_kek_id, ContentKeyring, WrappedKey};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::content_keys::{open_joined, JoinedWorkspaceKey, KeyState, Sealing};
use crate::error::StoreError;

/// The key row for `subject`, with the workspace key that wraps it. SQLite
/// serializes writers, so the append's transaction already excludes a
/// concurrent shred.
pub(crate) async fn state_for_update(
    tx: &mut Transaction<'_, Sqlite>,
    subject: Uuid,
) -> Result<(KeyState, JoinedWorkspaceKey), StoreError> {
    let row = sqlx::query(
        "SELECT k.kek_id, k.wrapped_key,
                wk.wrapping_key_id AS wk_kek_id, wk.wrapped_key AS wk_wrapped
         FROM maidan_content_keys k
         LEFT JOIN maidan_workspace_keys wk ON wk.id = k.workspace_key_id
         WHERE k.id = ?",
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
/// when the workspace has none yet. SQLite serializes writers, so no other
/// transaction can add one between the read and the insert.
pub(crate) async fn active_workspace_key(
    tx: &mut Transaction<'_, Sqlite>,
    keys: &ContentKeyring,
    workspace_id: Uuid,
) -> Result<Uuid, StoreError> {
    let row = sqlx::query(
        "SELECT id, wrapping_key_id, wrapped_key FROM maidan_workspace_keys
         WHERE workspace_id = ? AND retired_at IS NULL",
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
    sqlx::query(
        "INSERT INTO maidan_workspace_keys (id, workspace_id, custody, wrapping_key_id, wrapped_key)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(keys.custody_name())
    .bind(&wrapped.kek_id)
    .bind(&wrapped.blob)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

/// Retire the workspace's active key and make a new one; see the Postgres
/// twin.
pub async fn rotate_workspace_key(
    pool: &SqlitePool,
    keys: &ContentKeyring,
    workspace_id: Uuid,
) -> Result<(Option<Uuid>, Uuid), StoreError> {
    let mut tx = pool.begin().await?;
    let retired: Option<Uuid> = sqlx::query_scalar(
        "UPDATE maidan_workspace_keys SET retired_at = ?
         WHERE workspace_id = ? AND retired_at IS NULL
         RETURNING id",
    )
    .bind(Utc::now())
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
    let active = active_workspace_key(&mut tx, keys, workspace_id).await?;
    tx.commit().await?;
    Ok((retired, active))
}

pub(crate) async fn insert_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    subject: Uuid,
    workspace_id: Uuid,
    wrapped: Option<&WrappedKey>,
) -> Result<(), StoreError> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_content_keys
             (id, workspace_id, kek_id, wrapped_key, workspace_key_id, created_at, shredded_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
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
    sqlx::query("DELETE FROM maidan_mail_outbox WHERE content_key_id = ?")
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

/// One rewrap batch; see the Postgres twin for the three kinds of row.
pub async fn rewrap(
    pool: &SqlitePool,
    keys: &ContentKeyring,
    limit: i64,
) -> Result<u64, StoreError> {
    let mut changed = 0u64;
    let mut tx = pool.begin().await?;

    let rows = sqlx::query(
        "SELECT id, wrapping_key_id, wrapped_key FROM maidan_workspace_keys
         WHERE destroyed_at IS NULL AND wrapping_key_id NOT LIKE ?
         ORDER BY id
         LIMIT ?",
    )
    .bind(keys.workspace_wrapping_like())
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
            "UPDATE maidan_workspace_keys SET custody = ?, wrapping_key_id = ?, wrapped_key = ?
             WHERE id = ?",
        )
        .bind(keys.custody_name())
        .bind(&rewrapped.kek_id)
        .bind(&rewrapped.blob)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    changed += rows.len() as u64;

    let rows = sqlx::query(
        "SELECT k.id, k.workspace_id, k.kek_id, k.wrapped_key,
                wk.wrapping_key_id AS wk_kek_id, wk.wrapped_key AS wk_wrapped
         FROM maidan_content_keys k
         LEFT JOIN maidan_workspace_keys wk ON wk.id = k.workspace_key_id
         WHERE k.shredded_at IS NULL
           AND (k.workspace_key_id IS NULL OR wk.retired_at IS NOT NULL)
         ORDER BY k.id
         LIMIT ?",
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
            "UPDATE maidan_content_keys SET kek_id = ?, wrapped_key = ?, workspace_key_id = ?
             WHERE id = ?",
        )
        .bind(&rewrapped.kek_id)
        .bind(&rewrapped.blob)
        .bind(active)
        .bind(subject)
        .execute(&mut *tx)
        .await?;
    }
    changed += rows.len() as u64;

    // The custody destroys its key first, inside the transaction: a refusal
    // leaves the row (and the pass) to try again, and no row claims a
    // destroyed key it still holds.
    let doomed = sqlx::query(
        "SELECT r.id, r.wrapping_key_id, r.wrapped_key FROM maidan_workspace_keys r
         WHERE r.retired_at IS NOT NULL AND r.destroyed_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM maidan_content_keys k
               WHERE k.workspace_key_id = r.id AND k.shredded_at IS NULL
           )
         ORDER BY r.id
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let mut destroyed = Vec::with_capacity(doomed.len());
    for row in &doomed {
        let id: Uuid = row.get("id");
        if let (Some(kek_id), Some(blob)) = (
            row.get::<Option<String>, _>("wrapping_key_id"),
            row.get::<Option<Vec<u8>>, _>("wrapped_key"),
        ) {
            keys.destroy_workspace_key(id, &WrappedKey { kek_id, blob })
                .await?;
        }
        sqlx::query(
            "UPDATE maidan_workspace_keys
             SET wrapping_key_id = NULL, wrapped_key = NULL, destroyed_at = ?
             WHERE id = ?",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        destroyed.push(id);
    }
    tx.commit().await?;
    changed += destroyed.len() as u64;
    Ok(changed)
}

/// Rows the rewrap pass has still to change.
pub async fn count_needing_rewrap(
    pool: &SqlitePool,
    keys: &ContentKeyring,
) -> Result<u64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT
            (SELECT COUNT(*) FROM maidan_workspace_keys
             WHERE destroyed_at IS NULL AND wrapping_key_id NOT LIKE ?)
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
    .bind(keys.workspace_wrapping_like())
    .fetch_one(pool)
    .await?;
    Ok(count as u64)
}
