//! Per-message content keys on SQLite; see the Postgres twin.

use chrono::Utc;
use maidan_types::{parse_workspace_kek_id, ContentKeyring, WrappedKey};
use sqlx::{Acquire, Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::content_keys::{open_joined, JoinedWorkspaceKey, KeyState, Sealing};
use crate::error::StoreError;
use crate::store::{RewrapPass, StraySweep};

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
) -> Result<RewrapPass, StoreError> {
    let mut pass = RewrapPass::default();
    let mut after = Uuid::nil();
    loop {
        let mut tx = pool.begin().await?;
        let rows = sqlx::query(
            "SELECT id, wrapping_key_id, wrapped_key FROM maidan_workspace_keys
         WHERE destroyed_at IS NULL AND wrapping_key_id NOT LIKE ?
           AND id > ?
         ORDER BY id
         LIMIT ?",
        )
        .bind(keys.workspace_wrapping_like())
        .bind(after)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        let Some(last) = rows.last() else {
            break;
        };
        after = last.get("id");
        for row in &rows {
            // A savepoint per row: a row the custody refuses is rolled
            // back, logged and skipped, so one workspace never holds up
            // another's rewrap or the destroy step below.
            let mut sp = tx.begin().await?;
            let id: Uuid = row.get("id");
            let done: Result<(), StoreError> = async {
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
            .execute(&mut *sp)
            .await?;
                Ok(())
            }
            .await;
            match done {
                Ok(()) => {
                    sp.commit().await?;
                    pass.changed += 1;
                }
                Err(err) => {
                    tracing::warn!(row = %id, error = %err, "custody refused to rewrap a workspace key under the primary custody; skipped");
                    pass.skipped += 1;
                    sp.rollback().await?;
                }
            }
        }
        tx.commit().await?;
        if (rows.len() as i64) < limit {
            break;
        }
    }

    // Workspace keys the custody refused to open this pass: their other
    // rows are skipped without asking it again.
    let mut refused = std::collections::HashSet::<Uuid>::new();
    let mut after = Uuid::nil();
    loop {
        let mut tx = pool.begin().await?;
        let rows = sqlx::query(
            "SELECT k.id, k.workspace_id, k.workspace_key_id, k.kek_id, k.wrapped_key,
                wk.wrapping_key_id AS wk_kek_id, wk.wrapped_key AS wk_wrapped
         FROM maidan_content_keys k
         LEFT JOIN maidan_workspace_keys wk ON wk.id = k.workspace_key_id
         WHERE k.shredded_at IS NULL
           AND (k.workspace_key_id IS NULL OR wk.retired_at IS NOT NULL)
           AND k.id > ?
         ORDER BY k.id
         LIMIT ?",
        )
        .bind(after)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        let Some(last) = rows.last() else {
            break;
        };
        after = last.get("id");
        for row in &rows {
            // A savepoint per row: a row the custody refuses is rolled
            // back, logged and skipped, so one workspace never holds up
            // another's rewrap or the destroy step below.
            let id: Uuid = row.get("id");
            let workspace_key: Option<Uuid> = row.get("workspace_key_id");
            if workspace_key.is_some_and(|w| refused.contains(&w)) {
                pass.skipped += 1;
                continue;
            }
            let mut sp = tx.begin().await?;
            let mut open_refused = false;
            let done: Result<(), StoreError> = async {
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
                .await
                .inspect_err(|err| open_refused = err.is_custody_refusal())?;
                let data_key = keys.unwrap(subject, &wrapped)?;
                let active = active_workspace_key(&mut sp, keys, row.get("workspace_id")).await?;
                let rewrapped = keys.wrap_for_workspace(active, subject, &data_key)?;
                sqlx::query(
                "UPDATE maidan_content_keys SET kek_id = ?, wrapped_key = ?, workspace_key_id = ?
             WHERE id = ?",
            )
            .bind(&rewrapped.kek_id)
            .bind(&rewrapped.blob)
            .bind(active)
            .bind(subject)
            .execute(&mut *sp)
            .await?;
                Ok(())
            }
            .await;
            if open_refused && let Some(w) = workspace_key {
                refused.insert(w);
            }
            match done {
                Ok(()) => {
                    sp.commit().await?;
                    pass.changed += 1;
                }
                Err(err) => {
                    tracing::warn!(row = %id, error = %err, "custody refused to rewrap a data key under its workspace's active key; skipped");
                    pass.skipped += 1;
                    sp.rollback().await?;
                }
            }
        }
        tx.commit().await?;
        if (rows.len() as i64) < limit {
            break;
        }
    }

    // Each retired key is destroyed on its own, with no transaction or lock
    // held across the custody call: destroying is idempotent, so the row is
    // marked after its key is gone, and a key the custody refuses is logged
    // and skipped, so one workspace's refusal never stops another's.

    let mut after = Uuid::nil();
    loop {
        let doomed = sqlx::query(
            "SELECT r.id, r.wrapping_key_id, r.wrapped_key FROM maidan_workspace_keys r
         WHERE r.retired_at IS NOT NULL AND r.destroyed_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM maidan_content_keys k
               WHERE k.workspace_key_id = r.id AND k.shredded_at IS NULL
           )
           AND r.id > ?
         ORDER BY r.id
         LIMIT ?",
        )
        .bind(after)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        let Some(last) = doomed.last() else {
            break;
        };
        after = last.get("id");
        for row in &doomed {
            let id: Uuid = row.get("id");
            if let (Some(kek_id), Some(blob)) = (
                row.get::<Option<String>, _>("wrapping_key_id"),
                row.get::<Option<Vec<u8>>, _>("wrapped_key"),
            ) && let Err(err) = keys
                .destroy_workspace_key(id, &WrappedKey { kek_id, blob })
                .await
            {
                tracing::warn!(workspace_key = %id, error = %err, "custody refused to destroy a retired workspace key; skipped");
                pass.skipped += 1;
                pass.undestroyed += 1;
                continue;
            }
            let marked = sqlx::query(
                "UPDATE maidan_workspace_keys
             SET wrapping_key_id = NULL, wrapped_key = NULL, destroyed_at = ?
             WHERE id = ? AND destroyed_at IS NULL",
            )
            .bind(Utc::now())
            .bind(id)
            .execute(pool)
            .await?;
            pass.changed += marked.rows_affected();
        }
        if (doomed.len() as i64) < limit {
            break;
        }
    }
    Ok(pass)
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

/// Destroy custody keys older than `grace` (by this server's clock) that no
/// workspace key row in any workspace refers to; see [`crate::EventStore::sweep_stray_custody_keys`]. Each is
/// checked and destroyed on its own, with no transaction held across the
/// custody call, and a refusal is logged and counted, never fatal.
pub async fn sweep_stray(
    pool: &SqlitePool,
    keys: &ContentKeyring,
    grace: std::time::Duration,
) -> Result<StraySweep, StoreError> {
    let mut sweep = StraySweep::default();
    let Some(created_before) = chrono::Duration::from_std(grace)
        .ok()
        .and_then(|grace| Utc::now().checked_sub_signed(grace))
    else {
        return Ok(sweep);
    };
    for key in keys.custody_keys_before(created_before).await? {
        let referenced: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM maidan_workspace_keys WHERE wrapping_key_id = ?)",
        )
        .bind(&key.wrapping_key_id)
        .fetch_one(pool)
        .await?;
        if referenced {
            continue;
        }
        match keys.destroy_stray_custody_key(&key).await {
            Ok(()) => sweep.destroyed += 1,
            Err(err) => {
                tracing::warn!(workspace_key = %key.workspace_key_id, error = %err, "custody refused to destroy a stray key; retrying next sweep");
                sweep.failed += 1;
            }
        }
    }
    Ok(sweep)
}
