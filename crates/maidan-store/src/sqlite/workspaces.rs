use chrono::{DateTime, Utc};
use maidan_types::{NewWorkspace, Workspace, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &SqlitePool, new: NewWorkspace) -> Result<Workspace, StoreError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_workspaces (id, name, created_at, updated_at)
         VALUES (?, ?, ?, ?)
         RETURNING id, name, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(&new.name)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_workspace(&row))
}

pub async fn get(pool: &SqlitePool, id: WorkspaceId) -> Result<Workspace, StoreError> {
    let row = sqlx::query(
        "SELECT id, name, created_at, updated_at, tombstoned_at
         FROM maidan_workspaces WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_workspace(&row))
}

fn row_to_workspace(row: &sqlx::sqlite::SqliteRow) -> Workspace {
    Workspace {
        id: WorkspaceId(row.get::<Uuid, _>("id")),
        name: row.get("name"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    }
}
