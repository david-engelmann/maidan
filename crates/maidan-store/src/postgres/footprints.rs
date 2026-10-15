//! Claim footprints on Postgres; the rules are in [`crate::footprints`].
//!
//! Claims and footprint changes in a workspace that uses footprints take one
//! transaction-scoped advisory lock per workspace, so two claimers cannot each
//! see the other's overlapping work as free before either commits.

use maidan_types::{
    ActiveClaim, Footprint, FootprintConflict, MemberId, NewAuditEvent, RepoPause, SlotUsage,
    ThreadId, WorkspaceId, DM_CHANNEL_NAME,
};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::claim_next::ClaimScope;
use crate::error::StoreError;
use crate::footprints::{
    conflict_from_row, footprint_from_rows, group_active, live_claim, ItemRow,
};
use crate::AuditFor;

fn item_row(row: &sqlx::postgres::PgRow) -> ItemRow {
    ItemRow {
        thread_id: row.get("thread_id"),
        assignee_id: row.try_get("assignee_id").ok().flatten(),
        lease_expires_at: row.try_get("assignment_expires_at").ok().flatten(),
        repo: row.get("repo"),
        mode: row.get("mode"),
        kind: row.get("kind"),
        value: row.get("value"),
        task_kind: row.try_get("task_kind").ok().flatten(),
        task_engine: row.try_get("task_engine").ok().flatten(),
        threat_model: row.try_get("threat_model").ok().flatten(),
    }
}

async fn lock_workspace(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: Uuid,
) -> Result<(), StoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 1168))")
        .bind(format!("maidan.footprints.{workspace_id}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Before a claim in `scope`: serialize with other claims and footprint
/// changes in the workspace, when it has footprints at all. A workspace that
/// declares none claims exactly as before.
pub(crate) async fn lock_for_claim(
    tx: &mut Transaction<'_, Postgres>,
    scope: ClaimScope,
) -> Result<(), StoreError> {
    let workspace_id: Option<Uuid> = match scope {
        ClaimScope::Workspace(id) => Some(id.0),
        ClaimScope::Channel(id) => {
            sqlx::query_scalar("SELECT workspace_id FROM maidan_channels WHERE id = $1")
                .bind(id.0)
                .fetch_optional(&mut **tx)
                .await?
        }
    };
    let Some(workspace_id) = workspace_id else {
        return Ok(());
    };
    let declared: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM maidan_thread_footprints WHERE workspace_id = $1)",
    )
    .bind(workspace_id)
    .fetch_one(&mut **tx)
    .await?;
    if declared {
        lock_workspace(tx, workspace_id).await?;
    }
    Ok(())
}

/// Replace `thread_id`'s footprint (`None` clears it). Returns the live claims
/// it would overlap when the thread is itself claimed, and then stores
/// nothing; an empty list means it was stored.
pub async fn set(
    pool: &PgPool,
    thread_id: ThreadId,
    footprint: Option<Footprint>,
) -> Result<Vec<FootprintConflict>, StoreError> {
    let footprint = footprint
        .map(|footprint| footprint.validated().map_err(StoreError::InvalidInput))
        .transpose()?
        .filter(|footprint| !footprint.items.is_empty());
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "SELECT ch.workspace_id, {} AS live
         FROM maidan_threads t JOIN maidan_channels ch ON ch.id = t.channel_id
         WHERE t.id = $1
         FOR UPDATE OF t",
        live_claim("t", "NOW()")
    ))
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let workspace_id: Uuid = row.get("workspace_id");
    let live: bool = row.get("live");
    lock_workspace(&mut tx, workspace_id).await?;
    sqlx::query("DELETE FROM maidan_thread_footprints WHERE thread_id = $1")
        .bind(thread_id.0)
        .execute(&mut *tx)
        .await?;
    let Some(footprint) = footprint else {
        tx.commit().await?;
        return Ok(Vec::new());
    };
    insert_items_on(&mut tx, thread_id, workspace_id, &footprint).await?;
    if live {
        let rows = sqlx::query(&crate::footprints::conflicts_select("$1", "NOW()"))
            .bind(thread_id.0)
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
pub async fn get(pool: &PgPool, thread_id: ThreadId) -> Result<Option<Footprint>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, repo, mode, kind, value FROM maidan_thread_footprints
         WHERE thread_id = $1
         ORDER BY kind, value",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    footprint_from_rows(rows.iter().map(item_row).collect())
}

/// The live claims in `workspace_id` with their footprints, in `repo` when
/// given, limited to threads `readable_by` may read when given.
pub async fn list_active(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: Option<String>,
    readable_by: Option<MemberId>,
) -> Result<Vec<ActiveClaim>, StoreError> {
    let repo = repo
        .map(|repo| maidan_types::normalize_repo(&repo).map_err(StoreError::InvalidInput))
        .transpose()?;
    let readable = crate::thread_access::readable_thread("held.id", "$1", "$3::uuid", "$4");
    let rows = sqlx::query(&format!(
        "SELECT f.thread_id, held.assignee_id, held.assignment_expires_at,
                f.repo, f.mode, f.kind, f.value,
                a.kind AS task_kind, a.engine AS task_engine,
                a.threat_model AS threat_model
         FROM maidan_thread_footprints f
         JOIN maidan_threads held ON held.id = f.thread_id
         LEFT JOIN maidan_thread_attributes a ON a.thread_id = f.thread_id
         WHERE f.workspace_id = $1
           AND ($2::text IS NULL OR f.repo = $2)
           AND {}
           AND ($3::uuid IS NULL OR {readable})
         ORDER BY held.assignment_expires_at ASC NULLS LAST, f.thread_id, f.kind, f.value",
        live_claim("held", "NOW()")
    ))
    .bind(workspace_id.0)
    .bind(repo)
    .bind(readable_by.map(|m| m.0))
    .bind(DM_CHANNEL_NAME)
    .fetch_all(pool)
    .await?;
    group_active(rows.iter().map(item_row).collect())
}

fn pause_row(row: &sqlx::postgres::PgRow) -> RepoPause {
    RepoPause {
        workspace_id: WorkspaceId(row.get("workspace_id")),
        repo: row.get("repo"),
        paused_at: row.get("paused_at"),
        paused_by: MemberId(row.get("paused_by")),
        reason: row.get("reason"),
    }
}

const PAUSE_COLS: &str = "workspace_id, repo, paused_at, paused_by, reason";

/// Pause `repo` (already normalized) in `workspace_id`, or restate a pause
/// with a new reason; the audit row commits with it (D-A).
pub async fn pause_repo(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: &str,
    paused_by: MemberId,
    reason: Option<&str>,
    audit_for: AuditFor<RepoPause>,
) -> Result<RepoPause, StoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_repo_pauses (workspace_id, repo, paused_at, paused_by, reason)
         VALUES ($1, $2, NOW(), $3, $4)
         ON CONFLICT (workspace_id, repo) DO UPDATE SET
             paused_at = excluded.paused_at, paused_by = excluded.paused_by,
             reason = excluded.reason
         RETURNING {PAUSE_COLS}"
    ))
    .bind(workspace_id.0)
    .bind(repo)
    .bind(paused_by.0)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    let pause = pause_row(&row);
    super::audit::append_counted(&mut tx, audit_for(&pause)).await?;
    tx.commit().await?;
    Ok(pause)
}

/// Lift the pause on `repo` in `workspace_id`. `false`, and nothing
/// recorded, when it was not paused.
pub async fn resume_repo(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: &str,
    audit: NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let removed =
        sqlx::query("DELETE FROM maidan_repo_pauses WHERE workspace_id = $1 AND repo = $2")
            .bind(workspace_id.0)
            .bind(repo)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
    if removed {
        super::audit::append_counted(&mut tx, audit).await?;
    }
    tx.commit().await?;
    Ok(removed)
}

/// The repositories paused in `workspace_id`, by repository.
pub async fn list_repo_pauses(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<RepoPause>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {PAUSE_COLS} FROM maidan_repo_pauses WHERE workspace_id = $1 ORDER BY repo"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(pause_row).collect())
}

/// The slots in use or with a capacity in `workspace_id`, by name.
pub async fn list_slots(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<SlotUsage>, StoreError> {
    let mut slots: std::collections::BTreeMap<String, SlotUsage> =
        std::collections::BTreeMap::new();
    let capacities =
        sqlx::query("SELECT slot, capacity FROM maidan_slot_capacities WHERE workspace_id = $1")
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
         WHERE f.workspace_id = $1 AND f.match_namespace = 'slot' AND {}
         GROUP BY f.match_key",
        live_claim("held", "NOW()")
    ))
    .bind(workspace_id.0)
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
    slot: &str,
    capacity: i64,
    updated_by: MemberId,
    audit: NewAuditEvent,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO maidan_slot_capacities (workspace_id, slot, capacity, updated_at, updated_by)
         VALUES ($1, $2, $3, NOW(), $4)
         ON CONFLICT (workspace_id, slot) DO UPDATE SET
             capacity = excluded.capacity, updated_at = excluded.updated_at,
             updated_by = excluded.updated_by",
    )
    .bind(workspace_id.0)
    .bind(slot)
    .bind(capacity)
    .bind(updated_by.0)
    .execute(&mut *tx)
    .await?;
    super::audit::append_counted(&mut tx, audit).await?;
    tx.commit().await?;
    Ok(())
}

/// Remove `slot`'s capacity, so it limits nothing. `false`, and nothing
/// recorded, when it had none.
pub async fn clear_slot_capacity(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    slot: &str,
    audit: NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let removed =
        sqlx::query("DELETE FROM maidan_slot_capacities WHERE workspace_id = $1 AND slot = $2")
            .bind(workspace_id.0)
            .bind(slot)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
    if removed {
        super::audit::append_counted(&mut tx, audit).await?;
    }
    tx.commit().await?;
    Ok(removed)
}

/// Store `footprint`'s items for `thread_id` on the caller's connection. The
/// footprint is already validated and the thread has none.
pub(crate) async fn insert_items_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
    workspace_id: Uuid,
    footprint: &Footprint,
) -> Result<(), StoreError> {
    for item in &footprint.items {
        sqlx::query(
            "INSERT INTO maidan_thread_footprints
                 (thread_id, workspace_id, repo, mode, kind, value, match_namespace, match_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
    readable_by: Option<MemberId>,
) -> Result<Vec<maidan_types::DeferredThread>, StoreError> {
    let rows = sqlx::query(&crate::footprints::deferred_select(
        "$1", "$2::uuid", "$3", "NOW()",
    ))
    .bind(workspace_id.0)
    .bind(readable_by.map(|m| m.0))
    .bind(DM_CHANNEL_NAME)
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
