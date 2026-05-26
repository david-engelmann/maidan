use chrono::{DateTime, Utc};
use maidan_types::{NewWorkspace, Workspace, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &PgPool, new: NewWorkspace) -> Result<Workspace, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_workspaces (id, name)
         VALUES ($1, $2)
         RETURNING id, name, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(&new.name)
    .fetch_one(pool)
    .await?;
    Ok(row_to_workspace(&row))
}

pub async fn count(pool: &PgPool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COUNT(*)::bigint AS n FROM maidan_workspaces")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("n"))
}

pub async fn get(pool: &PgPool, id: WorkspaceId) -> Result<Workspace, StoreError> {
    let row = sqlx::query(
        "SELECT id, name, created_at, updated_at, tombstoned_at
         FROM maidan_workspaces WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_workspace(&row))
}

fn row_to_workspace(row: &sqlx::postgres::PgRow) -> Workspace {
    Workspace {
        id: WorkspaceId(row.get::<Uuid, _>("id")),
        name: row.get("name"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    }
}
