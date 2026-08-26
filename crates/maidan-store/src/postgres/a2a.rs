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

pub async fn list_tasks(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<serde_json::Value>, StoreError> {
    let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
        "SELECT task_json FROM maidan_a2a_tasks WHERE workspace_id = $1
         ORDER BY updated_at DESC, id DESC LIMIT $2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

pub async fn create_task_push_config(
    pool: &PgPool,
    task_id: &str,
    config_id: &str,
    url: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_a2a_task_push_configs (task_id, config_id, push_url)
         VALUES ($1, $2, $3)
         ON CONFLICT (task_id, config_id) DO UPDATE SET push_url = EXCLUDED.push_url",
    )
    .bind(task_id)
    .bind(config_id)
    .bind(url)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_task_push_config(
    pool: &PgPool,
    task_id: &str,
    config_id: &str,
) -> Result<Option<String>, StoreError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT push_url FROM maidan_a2a_task_push_configs WHERE task_id = $1 AND config_id = $2",
    )
    .bind(task_id)
    .bind(config_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

pub async fn list_task_push_configs(
    pool: &PgPool,
    task_id: &str,
) -> Result<Vec<(String, String)>, StoreError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT config_id, push_url FROM maidan_a2a_task_push_configs
         WHERE task_id = $1 ORDER BY created_at ASC, config_id ASC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_task_push_config(
    pool: &PgPool,
    task_id: &str,
    config_id: &str,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_a2a_task_push_configs WHERE task_id = $1 AND config_id = $2",
    )
    .bind(task_id)
    .bind(config_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}
