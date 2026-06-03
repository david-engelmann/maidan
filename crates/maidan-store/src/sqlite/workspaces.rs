use chrono::{DateTime, Utc};
use maidan_types::{NewWorkspace, WebhookSubscriptionId, Workspace, WorkspaceId};
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

pub async fn count(pool: &SqlitePool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM maidan_workspaces")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("n"))
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

pub async fn get_mention_webhook_id(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Option<WebhookSubscriptionId>, StoreError> {
    let id: Option<Uuid> =
        sqlx::query_scalar("SELECT mention_webhook_id FROM maidan_workspaces WHERE id = ?")
            .bind(workspace_id.0)
            .fetch_optional(pool)
            .await?
            .ok_or(StoreError::NotFound)?;
    Ok(id.map(WebhookSubscriptionId))
}

pub async fn set_mention_webhook_id(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    webhook_id: Option<WebhookSubscriptionId>,
) -> Result<(), StoreError> {
    let updated = sqlx::query("UPDATE maidan_workspaces SET mention_webhook_id = ? WHERE id = ?")
        .bind(webhook_id.map(|w| w.0))
        .bind(workspace_id.0)
        .execute(pool)
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
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
