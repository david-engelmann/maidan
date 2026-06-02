use maidan_types::WorkspaceId;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::StoreError;

pub async fn upsert_push_config(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    push_url: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_a2a_push_configs (workspace_id, push_url)
         VALUES ($1, $2)
         ON CONFLICT (workspace_id) DO UPDATE SET
            push_url = EXCLUDED.push_url,
            updated_at = NOW()",
    )
    .bind(workspace_id.0)
    .bind(push_url)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_push_config(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<String>, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT push_url FROM maidan_a2a_push_configs WHERE workspace_id = $1")
            .bind(workspace_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

pub async fn upsert_task(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    task_id: &str,
    task_json: serde_json::Value,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_a2a_tasks (id, workspace_id, task_json)
         VALUES ($1, $2, $3)
         ON CONFLICT (id) DO UPDATE SET
            task_json = EXCLUDED.task_json,
            updated_at = NOW()",
    )
    .bind(task_id)
    .bind(workspace_id.0)
    .bind(task_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_task(
    pool: &PgPool,
    task_id: &str,
) -> Result<Option<serde_json::Value>, StoreError> {
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT task_json FROM maidan_a2a_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

pub async fn get_task_workspace(
    pool: &PgPool,
    task_id: &str,
) -> Result<Option<WorkspaceId>, StoreError> {
    let row: Option<Uuid> =
        sqlx::query_scalar("SELECT workspace_id FROM maidan_a2a_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(WorkspaceId))
}
