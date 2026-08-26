use maidan_types::WorkspaceId;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StoreError;

pub async fn upsert_push_config(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    push_url: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_a2a_push_configs (workspace_id, push_url)
         VALUES (?, ?)
         ON CONFLICT(workspace_id) DO UPDATE SET
            push_url = excluded.push_url,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
    )
    .bind(workspace_id.0)
    .bind(push_url)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_push_config(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Option<String>, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT push_url FROM maidan_a2a_push_configs WHERE workspace_id = ?")
            .bind(workspace_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

pub async fn upsert_task(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    task_id: &str,
    task_json: serde_json::Value,
) -> Result<(), StoreError> {
    let json = serde_json::to_string(&task_json)?;
    sqlx::query(
        "INSERT INTO maidan_a2a_tasks (id, workspace_id, task_json)
         VALUES (?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            task_json = excluded.task_json,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
    )
    .bind(task_id)
    .bind(workspace_id.0)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_task(
    pool: &SqlitePool,
    task_id: &str,
) -> Result<Option<serde_json::Value>, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT task_json FROM maidan_a2a_tasks WHERE id = ?")
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    row.map(|r| serde_json::from_str(&r.0).map_err(StoreError::from))
        .transpose()
}

pub async fn get_task_workspace(
    pool: &SqlitePool,
    task_id: &str,
) -> Result<Option<WorkspaceId>, StoreError> {
    let row: Option<Uuid> =
        sqlx::query_scalar("SELECT workspace_id FROM maidan_a2a_tasks WHERE id = ?")
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(WorkspaceId))
}

pub async fn list_tasks(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<serde_json::Value>, StoreError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT task_json FROM maidan_a2a_tasks WHERE workspace_id = ?
         ORDER BY updated_at DESC, id DESC LIMIT ?",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| serde_json::from_str(&r.0).map_err(StoreError::from))
        .collect()
}
