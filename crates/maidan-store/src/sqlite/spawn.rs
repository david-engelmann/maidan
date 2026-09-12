//! Spawn-budget store (Cluster 376, Wave 2 #23, SQLite twin of pg 0080): the
//! per-workspace caps + the spawn-time counts the gate reads (children, depth,
//! recorded tool-use).

use chrono::{DateTime, Utc};
use maidan_types::{SpawnBudget, ThreadId, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_budget(row: &sqlx::sqlite::SqliteRow) -> SpawnBudget {
    SpawnBudget {
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        max_children: row.get::<Option<i64>, _>("max_children"),
        max_depth: row.get::<Option<i64>, _>("max_depth"),
        max_tools: row.get::<Option<i64>, _>("max_tools"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

pub async fn set_budget(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    max_children: Option<i64>,
    max_depth: Option<i64>,
    max_tools: Option<i64>,
) -> Result<Option<SpawnBudget>, StoreError> {
    if max_children.is_none() && max_depth.is_none() && max_tools.is_none() {
        sqlx::query("DELETE FROM maidan_spawn_budgets WHERE workspace_id = ?")
            .bind(workspace_id.0)
            .execute(pool)
            .await?;
        return Ok(None);
    }
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_spawn_budgets (workspace_id, max_children, max_depth, max_tools, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id) DO UPDATE SET
             max_children = excluded.max_children, max_depth = excluded.max_depth,
             max_tools = excluded.max_tools, updated_at = excluded.updated_at
         RETURNING workspace_id, max_children, max_depth, max_tools, updated_at",
    )
    .bind(workspace_id.0)
    .bind(max_children)
    .bind(max_depth)
    .bind(max_tools)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(Some(row_to_budget(&row)))
}

pub async fn get_budget(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Option<SpawnBudget>, StoreError> {
    let row = sqlx::query(
        "SELECT workspace_id, max_children, max_depth, max_tools, updated_at
         FROM maidan_spawn_budgets WHERE workspace_id = ?",
    )
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_budget))
}

pub async fn count_active_children(
    pool: &SqlitePool,
    parent_thread_id: ThreadId,
) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM maidan_threads
         WHERE parent_thread_id = ? AND tombstoned_at IS NULL",
    )
    .bind(parent_thread_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n"))
}

pub async fn thread_depth(pool: &SqlitePool, thread_id: ThreadId) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "WITH RECURSIVE anc(id, parent, d) AS (
             SELECT id, parent_thread_id, 1 FROM maidan_threads WHERE id = ?
             UNION ALL
             SELECT t.id, t.parent_thread_id, anc.d + 1
             FROM maidan_threads t JOIN anc ON t.id = anc.parent
         )
         SELECT COALESCE(MAX(d), 0) AS depth FROM anc",
    )
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("depth"))
}

pub async fn count_tool_uses(pool: &SqlitePool, thread_id: ThreadId) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n
         FROM (
             SELECT content FROM maidan_messages
             WHERE thread_id = ? AND tombstoned_at IS NULL
               AND content IS NOT NULL AND json_type(content) = 'array'
         ) m,
         json_each(m.content) AS blk
         WHERE json_extract(blk.value, '$.type') = 'tool_use'",
    )
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n"))
}
