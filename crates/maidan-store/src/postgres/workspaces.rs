use chrono::{DateTime, Utc};
use maidan_types::{
    Event, NewWorkspace, StoredEvent, WebhookSubscriptionId, Workspace, WorkspaceId, WorkspaceUsage,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::postgres::events;

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

/// Insert a workspace and append its `WorkspaceCreated` event in one transaction
/// (Cluster 213) — see the SQLite twin.
pub async fn create_with_event(
    pool: &PgPool,
    new: NewWorkspace,
) -> Result<(Workspace, StoredEvent), StoreError> {
    let id = Uuid::new_v4();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO maidan_workspaces (id, name)
         VALUES ($1, $2)
         RETURNING id, name, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(&new.name)
    .fetch_one(&mut *tx)
    .await?;
    let workspace = row_to_workspace(&row);
    let event = Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: workspace.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((workspace, stored))
}

pub async fn count(pool: &PgPool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COUNT(*)::bigint AS n FROM maidan_workspaces")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("n"))
}

pub async fn usage(pool: &PgPool, id: WorkspaceId) -> Result<WorkspaceUsage, StoreError> {
    let row = sqlx::query(
        "SELECT
           (SELECT COUNT(*)::bigint FROM maidan_members
              WHERE workspace_id = $1 AND tombstoned_at IS NULL) AS members,
           (SELECT COUNT(*)::bigint FROM maidan_channels
              WHERE workspace_id = $1 AND tombstoned_at IS NULL) AS channels,
           (SELECT COUNT(*)::bigint FROM maidan_threads t
              JOIN maidan_channels c ON t.channel_id = c.id
              WHERE c.workspace_id = $1 AND t.tombstoned_at IS NULL) AS threads,
           (SELECT COUNT(*)::bigint FROM maidan_messages m
              JOIN maidan_threads t ON m.thread_id = t.id
              JOIN maidan_channels c ON t.channel_id = c.id
              WHERE c.workspace_id = $1 AND m.tombstoned_at IS NULL) AS messages",
    )
    .bind(id.0)
    .fetch_one(pool)
    .await?;
    Ok(WorkspaceUsage {
        workspace_id: id,
        members: row.get::<i64, _>("members"),
        channels: row.get::<i64, _>("channels"),
        threads: row.get::<i64, _>("threads"),
        messages: row.get::<i64, _>("messages"),
    })
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

pub async fn get_mention_webhook_id(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<WebhookSubscriptionId>, StoreError> {
    let id: Option<Uuid> =
        sqlx::query_scalar("SELECT mention_webhook_id FROM maidan_workspaces WHERE id = $1")
            .bind(workspace_id.0)
            .fetch_optional(pool)
            .await?
            .ok_or(StoreError::NotFound)?;
    Ok(id.map(WebhookSubscriptionId))
}

pub async fn set_mention_webhook_id(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    webhook_id: Option<WebhookSubscriptionId>,
) -> Result<(), StoreError> {
    let updated = sqlx::query("UPDATE maidan_workspaces SET mention_webhook_id = $2 WHERE id = $1")
        .bind(workspace_id.0)
        .bind(webhook_id.map(|w| w.0))
        .execute(pool)
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
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
