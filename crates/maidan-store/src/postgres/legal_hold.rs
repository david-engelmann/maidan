//! Legal-hold queries (Cluster 366, T6): the `maidan_legal_holds` table. A
//! workspace with a row here is under hold; the retention prune SQL (`retention.rs`)
//! reads this table directly to exempt held workspaces' events and to freeze audit
//! pruning.

use chrono::{DateTime, Utc};
use maidan_types::{LegalHold, MemberId, WorkspaceId};
use sqlx::{PgPool, Row};
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
    .fetch_one(pool)
    .await?;
    Ok(row_to_hold(&row))
}

pub async fn lift(pool: &PgPool, workspace_id: WorkspaceId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_legal_holds WHERE workspace_id = $1")
        .bind(workspace_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
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
