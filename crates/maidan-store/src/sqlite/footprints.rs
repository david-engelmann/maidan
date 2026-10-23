//! Claim footprints on SQLite; see the Postgres twin. SQLite serializes
//! writers, so a claim and a footprint change cannot interleave and no lock
//! is taken.

use chrono::Utc;
use maidan_types::{
    ActiveClaim, Footprint, FootprintConflict, MemberId, NewAuditEvent, RepoPause, SlotUsage,
    ThreadId, WorkspaceId, DM_CHANNEL_NAME,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::footprints::{
    conflict_from_row, footprint_from_rows, group_active, live_claim, ItemRow,
};
use crate::AuditFor;

fn item_row(row: &sqlx::sqlite::SqliteRow) -> ItemRow {
    ItemRow {
        thread_id: row.get("thread_id"),
        assignee_id: row.try_get("assignee_id").ok().flatten(),
        lease_expires_at: row
            .try_get::<Option<String>, _>("assignment_expires_at")
            .ok()
            .flatten()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(&at).ok())
            .map(|at| at.with_timezone(&Utc)),
        repo: row.get("repo"),
        mode: row.get("mode"),
        kind: row.get("kind"),
        value: row.get("value"),
        task_kind: row.try_get("task_kind").ok().flatten(),
        task_engine: row.try_get("task_engine").ok().flatten(),
        threat_model: row.try_get("threat_model").ok().flatten(),
        task_size: row.try_get("task_size").ok().flatten(),
        task_acceptance: row.try_get("task_acceptance").ok().flatten(),
        task_tier: row.try_get("task_tier").ok().flatten(),
    }
}

/// Replace `thread_id`'s footprint; see the Postgres twin.
pub async fn set(
    pool: &SqlitePool,
    thread_id: ThreadId,
    footprint: Option<Footprint>,
) -> Result<Vec<FootprintConflict>, StoreError> {
    let footprint = footprint
        .map(|footprint| footprint.validated().map_err(StoreError::InvalidInput))
        .transpose()?
        .filter(|footprint| !footprint.items.is_empty());
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "SELECT ch.workspace_id, {} AS live
         FROM maidan_threads t JOIN maidan_channels ch ON ch.id = t.channel_id
         WHERE t.id = ?1",
        live_claim("t", "?2")
    ))
    .bind(thread_id.0)
    .bind(&now)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let workspace_id: Uuid = row.get("workspace_id");
    let live: bool = row.get("live");
    sqlx::query("DELETE FROM maidan_thread_footprints WHERE thread_id = ?")
        .bind(thread_id.0)
        .execute(&mut *tx)
        .await?;
    let Some(footprint) = footprint else {
        tx.commit().await?;
        return Ok(Vec::new());
    };
    insert_items_on(&mut tx, thread_id, workspace_id, &footprint).await?;
    if live {
        let rows = sqlx::query(&crate::footprints::conflicts_select("?1", "?2"))
            .bind(thread_id.0)
            .bind(&now)
            .fetch_all(&mut *tx)
            .await?;
        if !rows.is_empty() {
            tx.rollback().await?;
            return rows
                .iter()
                .map(|row| conflict_from_row(item_row(row)))
                .collect();
        }
    }
    tx.commit().await?;
    Ok(Vec::new())
}

/// `thread_id`'s declared footprint, claimed or not.
pub async fn get(pool: &SqlitePool, thread_id: ThreadId) -> Result<Option<Footprint>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, repo, mode, kind, value FROM maidan_thread_footprints
         WHERE thread_id = ?
         ORDER BY kind, value",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    footprint_from_rows(rows.iter().map(item_row).collect())
}

/// The live claims in `workspace_id` with their footprints; see the Postgres
/// twin.
pub async fn list_active(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    repo: Option<String>,
    readable_by: Option<MemberId>,
) -> Result<Vec<ActiveClaim>, StoreError> {
    let repo = repo
        .map(|repo| maidan_types::normalize_repo(&repo).map_err(StoreError::InvalidInput))
        .transpose()?;
    let readable = crate::thread_access::readable_thread("held.id", "?1", "?3", "?4");
    let rows = sqlx::query(&format!(
        "SELECT f.thread_id, held.assignee_id, held.assignment_expires_at,
                f.repo, f.mode, f.kind, f.value,
                a.kind AS task_kind, a.engine AS task_engine,
                a.threat_model AS threat_model, a.size AS task_size, a.acceptance AS task_acceptance,
                a.tier AS task_tier
         FROM maidan_thread_footprints f
         JOIN maidan_threads held ON held.id = f.thread_id
         LEFT JOIN maidan_thread_attributes a ON a.thread_id = f.thread_id
         WHERE f.workspace_id = ?1
           AND (?2 IS NULL OR f.repo = ?2)
           AND {}
           AND (?3 IS NULL OR {readable})
         ORDER BY held.assignment_expires_at IS NULL, held.assignment_expires_at,
                  f.thread_id, f.kind, f.value",
        live_claim("held", "?5")
    ))
    .bind(workspace_id.0)
    .bind(repo)
    .bind(readable_by.map(|m| m.0))
    .bind(DM_CHANNEL_NAME)
    .bind(Utc::now().to_rfc3339())
    .fetch_all(pool)
    .await?;
    group_active(rows.iter().map(item_row).collect())
}

fn pause_row(row: &sqlx::sqlite::SqliteRow) -> RepoPause {
    RepoPause {
        workspace_id: WorkspaceId(row.get("workspace_id")),
        repo: row.get("repo"),
        paused_at: row.get::<chrono::DateTime<Utc>, _>("paused_at"),
        paused_by: MemberId(row.get("paused_by")),
        reason: row.get("reason"),
    }
}

const PAUSE_COLS: &str = "workspace_id, repo, paused_at, paused_by, reason";

/// Pause `repo` in `workspace_id`; see the Postgres twin.
pub async fn pause_repo(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    repo: &str,
    paused_by: MemberId,
    reason: Option<&str>,
    audit_for: AuditFor<RepoPause>,
) -> Result<RepoPause, StoreError> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_repo_pauses (workspace_id, repo, paused_at, paused_by, reason)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id, repo) DO UPDATE SET
             paused_at = excluded.paused_at, paused_by = excluded.paused_by,
             reason = excluded.reason
         RETURNING {PAUSE_COLS}"
    ))
    .bind(workspace_id.0)
    .bind(repo)
    .bind(&now)
    .bind(paused_by.0)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    let pause = pause_row(&row);
    super::audit::append_scoped(&mut tx, workspace_id, audit_for(&pause)).await?;
    tx.commit().await?;
    Ok(pause)
}

/// Lift the pause on `repo`; see the Postgres twin.
pub async fn resume_repo(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    repo: &str,
    audit: NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let removed = sqlx::query("DELETE FROM maidan_repo_pauses WHERE workspace_id = ? AND repo = ?")
        .bind(workspace_id.0)
        .bind(repo)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        > 0;
    if removed {
        super::audit::append_scoped(&mut tx, workspace_id, audit).await?;
    }
    tx.commit().await?;
    Ok(removed)
}

/// The repositories paused in `workspace_id`, by repository.
pub async fn list_repo_pauses(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<RepoPause>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {PAUSE_COLS} FROM maidan_repo_pauses WHERE workspace_id = ? ORDER BY repo"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(pause_row).collect())
}

/// The slots in use or with a capacity in `workspace_id`, by name.
pub async fn list_slots(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<SlotUsage>, StoreError> {
    let mut slots: std::collections::BTreeMap<String, SlotUsage> =
        std::collections::BTreeMap::new();
    let capacities =
        sqlx::query("SELECT slot, capacity FROM maidan_slot_capacities WHERE workspace_id = ?")
            .bind(workspace_id.0)
            .fetch_all(pool)
            .await?;
    for row in &capacities {
        let slot: String = row.get("slot");
        slots.insert(
            slot.clone(),
            SlotUsage {
                slot,
                capacity: Some(row.get::<i64, _>("capacity")),
                in_use: 0,
            },
        );
    }
    let in_use = sqlx::query(&format!(
        "SELECT f.match_key AS slot, COUNT(DISTINCT f.thread_id) AS in_use
         FROM maidan_thread_footprints f
         JOIN maidan_threads held ON held.id = f.thread_id
         WHERE f.workspace_id = ?1 AND f.match_namespace = 'slot' AND {}
         GROUP BY f.match_key",
        live_claim("held", "?2")
    ))
    .bind(workspace_id.0)
    .bind(chrono::Utc::now().to_rfc3339())
    .fetch_all(pool)
    .await?;
    for row in &in_use {
        let slot: String = row.get("slot");
        let count: i64 = row.get("in_use");
        slots
            .entry(slot.clone())
            .or_insert(SlotUsage {
                slot,
                capacity: None,
                in_use: 0,
            })
            .in_use = count;
    }
    Ok(slots.into_values().collect())
}

/// Set `slot`'s capacity (already normalized) in `workspace_id`; the audit
/// row commits with it (D-A).
pub async fn set_slot_capacity(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    slot: &str,
    capacity: i64,
    updated_by: MemberId,
    audit: NewAuditEvent,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO maidan_slot_capacities (workspace_id, slot, capacity, updated_at, updated_by)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id, slot) DO UPDATE SET
             capacity = excluded.capacity, updated_at = excluded.updated_at,
             updated_by = excluded.updated_by",
    )
    .bind(workspace_id.0)
    .bind(slot)
    .bind(capacity)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(updated_by.0)
    .execute(&mut *tx)
    .await?;
    super::audit::append_scoped(&mut tx, workspace_id, audit).await?;
    tx.commit().await?;
    Ok(())
}

/// Remove `slot`'s capacity, so it limits nothing. `false`, and nothing
/// recorded, when it had none.
pub async fn clear_slot_capacity(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    slot: &str,
    audit: NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let removed =
        sqlx::query("DELETE FROM maidan_slot_capacities WHERE workspace_id = ? AND slot = ?")
            .bind(workspace_id.0)
            .bind(slot)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
    if removed {
        super::audit::append_scoped(&mut tx, workspace_id, audit).await?;
    }
    tx.commit().await?;
    Ok(removed)
}

/// Store `footprint`'s items for `thread_id` on the caller's connection. The
/// footprint is already validated and the thread has none.
pub(crate) async fn insert_items_on(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
    workspace_id: Uuid,
    footprint: &Footprint,
) -> Result<(), StoreError> {
    for item in &footprint.items {
        sqlx::query(
            "INSERT INTO maidan_thread_footprints
                 (thread_id, workspace_id, repo, mode, kind, value, match_namespace, match_key)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(thread_id.0)
        .bind(workspace_id)
        .bind(&footprint.repo)
        .bind(footprint.mode.as_str())
        .bind(item.kind.as_str())
        .bind(&item.value)
        .bind(item.kind.namespace())
        .bind(item.key())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The threads the claim queue defers in `workspace_id`, with why; only those
/// `readable_by` may read when given.
pub async fn list_deferred(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    readable_by: Option<MemberId>,
) -> Result<Vec<maidan_types::DeferredThread>, StoreError> {
    let rows = sqlx::query(&crate::footprints::deferred_select("?1", "?2", "?3", "?4"))
        .bind(workspace_id.0)
        .bind(readable_by.map(|m| m.0))
        .bind(DM_CHANNEL_NAME)
        .bind(Utc::now().to_rfc3339())
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|row| maidan_types::DeferredThread {
            thread_id: ThreadId(row.get("thread_id")),
            title: row.get("title"),
            reasons: crate::footprints::deferral_reasons(
                row.get("overlap"),
                row.get("repo_paused"),
                row.get("slot_full"),
            ),
        })
        .collect())
}
