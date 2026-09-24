//! Legal-hold queries: the `maidan_legal_holds` table. A workspace with a row
//! here is under hold; the retention prune SQL (`retention.rs`) reads this
//! table directly to exempt held workspaces' events and to freeze audit
//! pruning.

use chrono::{DateTime, Utc};
use maidan_types::{LegalHold, MemberId, WorkspaceId};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_hold(row: &sqlx::postgres::PgRow) -> LegalHold {
    LegalHold {
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        reason: row.get::<String, _>("reason"),
        placed_by: row.get::<Option<Uuid>, _>("placed_by").map(MemberId),
        placed_at: row.get::<DateTime<Utc>, _>("placed_at"),
    }
}

const COLS: &str = "workspace_id, reason, placed_by, placed_at";

pub async fn place(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    reason: &str,
    placed_by: Option<MemberId>,
) -> Result<LegalHold, StoreError> {
    let mut conn = pool.acquire().await?;
    place_on(&mut conn, workspace_id, reason, placed_by).await
}

pub(crate) async fn place_on(
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
    reason: &str,
    placed_by: Option<MemberId>,
) -> Result<LegalHold, StoreError> {
    // Serialize with a purge or erase of the same workspace, which takes this
    // lock before checking for a hold (`refuse_if_held`).
    sqlx::query("SELECT 1 FROM maidan_workspaces WHERE id = $1 FOR NO KEY UPDATE")
        .bind(workspace_id.0)
        .fetch_optional(&mut *conn)
        .await?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_legal_holds (workspace_id, reason, placed_by, placed_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (workspace_id) DO UPDATE SET
             reason = excluded.reason,
             placed_by = excluded.placed_by,
             placed_at = excluded.placed_at
         RETURNING {COLS}"
    ))
    .bind(workspace_id.0)
    .bind(reason)
    .bind(placed_by.map(|m| m.0))
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_hold(&row))
}

pub async fn lift(pool: &PgPool, workspace_id: WorkspaceId) -> Result<bool, StoreError> {
    let mut conn = pool.acquire().await?;
    lift_on(&mut conn, workspace_id).await
}

pub(crate) async fn lift_on(
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_legal_holds WHERE workspace_id = $1")
        .bind(workspace_id.0)
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Refuse to destroy a held workspace's data: `Conflict`, which the API
/// answers with 409. `NotFound` when the workspace does not exist. Run it in
/// the destroying transaction.
pub(crate) async fn refuse_if_held(
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
) -> Result<(), StoreError> {
    // Held until the transaction ends, so a hold placed meanwhile waits for it
    // (see `place_on`).
    let exists = sqlx::query("SELECT 1 FROM maidan_workspaces WHERE id = $1 FOR NO KEY UPDATE")
        .bind(workspace_id.0)
        .fetch_optional(&mut *conn)
        .await?
        .is_some();
    if !exists {
        return Err(StoreError::NotFound);
    }
    let held: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maidan_legal_holds WHERE workspace_id = $1)",
    )
    .bind(workspace_id.0)
    .fetch_one(&mut *conn)
    .await?;
    if held {
        return Err(StoreError::Conflict(crate::LEGAL_HOLD_REFUSAL.into()));
    }
    Ok(())
}

pub async fn get(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<LegalHold>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_legal_holds WHERE workspace_id = $1"
    ))
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_hold))
}

pub async fn list(pool: &PgPool) -> Result<Vec<LegalHold>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_legal_holds ORDER BY placed_at DESC, workspace_id ASC"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_hold).collect())
}
