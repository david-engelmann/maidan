//! Claim footprints on SQLite; see the Postgres twin. SQLite serializes
//! writers, so a claim and a footprint change cannot interleave and no lock
//! is taken.

use chrono::Utc;
use maidan_types::{
    ActiveClaim, Footprint, FootprintConflict, MemberId, NewAuditEvent, RepoPause, ThreadId,
    WorkspaceId, DM_CHANNEL_NAME,
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
        .execute(&mut *tx)
        .await?;
    }
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
                f.repo, f.mode, f.kind, f.value
         FROM maidan_thread_footprints f
         JOIN maidan_threads held ON held.id = f.thread_id
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
    super::audit::append_counted(&mut tx, audit_for(&pause)).await?;
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
        super::audit::append_counted(&mut tx, audit).await?;
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
