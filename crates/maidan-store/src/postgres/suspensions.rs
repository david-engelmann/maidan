//! Workspace suspensions (`maidan_types::suspension`). Suspending and lifting
//! write their audit row in the same transaction, so neither happens unrecorded.

use chrono::Utc;
use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{MemberId, NewAuditEvent, WorkspaceId, WorkspaceSuspension};

const COLS: &str = "workspace_id, reason, suspended_by, suspended_at";

/// Suspend `workspace_id`, or replace the reason of a standing suspension.
pub async fn suspend(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    reason: &str,
    suspended_by: Option<MemberId>,
    audit: NewAuditEvent,
) -> Result<WorkspaceSuspension, StoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_workspace_suspensions (workspace_id, reason, suspended_by, suspended_at)
         SELECT id, $2, $3, $4 FROM maidan_workspaces WHERE id = $1
         ON CONFLICT (workspace_id) DO UPDATE SET reason = EXCLUDED.reason
         RETURNING {COLS}"
    ))
    .bind(workspace_id.0)
    .bind(reason)
    .bind(suspended_by.map(|m| m.0))
    .bind(Utc::now())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let suspension = row_to_suspension(&row)?;
    super::audit::append_counted(&mut tx, audit).await?;
    tx.commit().await?;
    Ok(suspension)
}

/// Lift the suspension of `workspace_id`; `false` when there was none, and
/// then nothing is audited.
pub async fn lift(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    audit: NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let deleted = sqlx::query("DELETE FROM maidan_workspace_suspensions WHERE workspace_id = $1")
        .bind(workspace_id.0)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Ok(false);
    }
    super::audit::append_counted(&mut tx, audit).await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn get(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<WorkspaceSuspension>, StoreError> {
    sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_workspace_suspensions WHERE workspace_id = $1"
    ))
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?
    .as_ref()
    .map(row_to_suspension)
    .transpose()
}

fn row_to_suspension(row: &sqlx::postgres::PgRow) -> Result<WorkspaceSuspension, StoreError> {
    Ok(WorkspaceSuspension {
        workspace_id: WorkspaceId(row.try_get("workspace_id")?),
        reason: row.try_get("reason")?,
        suspended_by: row
            .try_get::<Option<uuid::Uuid>, _>("suspended_by")?
            .map(MemberId),
        suspended_at: row.try_get("suspended_at")?,
    })
}
