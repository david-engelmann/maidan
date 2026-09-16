//! Workspace handle aliases (Cluster 395). Separate table so a rename
//! cannot change stored workspace ids.

use chrono::{DateTime, Utc};
use maidan_types::{validate_workspace_handle, WorkspaceHandle, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn set(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    handle: &str,
) -> Result<WorkspaceHandle, StoreError> {
    validate_workspace_handle(handle).map_err(|e| StoreError::InvalidInput(e.to_string()))?;
    let row = sqlx::query(
        "INSERT INTO maidan_workspace_handles (workspace_id, handle)
         VALUES ($1, $2)
         ON CONFLICT (workspace_id) DO UPDATE
           SET handle = excluded.handle, updated_at = now()
         RETURNING workspace_id, handle, created_at, updated_at",
    )
    .bind(workspace_id.0)
    .bind(handle)
    .fetch_one(pool)
    .await
    .map_err(map_handle_err)?;
    Ok(row_to_handle(&row))
}

pub async fn get(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<WorkspaceHandle>, StoreError> {
    let row = sqlx::query(
        "SELECT workspace_id, handle, created_at, updated_at
         FROM maidan_workspace_handles WHERE workspace_id = $1",
    )
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_handle))
}

fn map_handle_err(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db) = err {
        if db.is_unique_violation() {
            return StoreError::Conflict("workspace handle already exists".into());
        }
        if db.is_foreign_key_violation() {
            return StoreError::NotFound;
        }
    }
    StoreError::Database(err)
}

fn row_to_handle(row: &sqlx::postgres::PgRow) -> WorkspaceHandle {
    WorkspaceHandle {
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        handle: row.get("handle"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}
