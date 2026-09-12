//! Spawn-budget store (Cluster 376, Wave 2 #23): the per-workspace caps
//! (`maidan_spawn_budgets`) + the spawn-time counts the gate reads — a parent's
//! direct children, a thread's nesting depth, and a thread's recorded tool-use
//! count. See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_types::{SpawnBudget, ThreadId, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_budget(row: &sqlx::postgres::PgRow) -> SpawnBudget {
    SpawnBudget {
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        max_children: row.get::<Option<i64>, _>("max_children"),
        max_depth: row.get::<Option<i64>, _>("max_depth"),
        max_tools: row.get::<Option<i64>, _>("max_tools"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

/// Upsert the caps, or clear the row when all three are `None` (fully unlimited).
pub async fn set_budget(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    max_children: Option<i64>,
    max_depth: Option<i64>,
    max_tools: Option<i64>,
) -> Result<Option<SpawnBudget>, StoreError> {
    if max_children.is_none() && max_depth.is_none() && max_tools.is_none() {
        sqlx::query("DELETE FROM maidan_spawn_budgets WHERE workspace_id = $1")
            .bind(workspace_id.0)
            .execute(pool)
            .await?;
        return Ok(None);
    }
    let row = sqlx::query(
        "INSERT INTO maidan_spawn_budgets (workspace_id, max_children, max_depth, max_tools, updated_at)
         VALUES ($1, $2, $3, $4, NOW())
         ON CONFLICT (workspace_id) DO UPDATE SET
             max_children = excluded.max_children, max_depth = excluded.max_depth,
             max_tools = excluded.max_tools, updated_at = NOW()
         RETURNING workspace_id, max_children, max_depth, max_tools, updated_at",
    )
    .bind(workspace_id.0)
    .bind(max_children)
    .bind(max_depth)
    .bind(max_tools)
    .fetch_one(pool)
    .await?;
    Ok(Some(row_to_budget(&row)))
}

pub async fn get_budget(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<SpawnBudget>, StoreError> {
    let row = sqlx::query(
        "SELECT workspace_id, max_children, max_depth, max_tools, updated_at
         FROM maidan_spawn_budgets WHERE workspace_id = $1",
    )
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_budget))
}

/// A parent's non-tombstoned direct children — the fan-out counted against
/// `max_children`.
pub async fn count_active_children(
    pool: &PgPool,
    parent_thread_id: ThreadId,
) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM maidan_threads
         WHERE parent_thread_id = $1 AND tombstoned_at IS NULL",
    )
    .bind(parent_thread_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n"))
}

/// A thread's nesting depth (a root thread is 1) via an ancestor walk.
pub async fn thread_depth(pool: &PgPool, thread_id: ThreadId) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "WITH RECURSIVE anc(id, parent, d) AS (
             SELECT id, parent_thread_id, CAST(1 AS BIGINT) FROM maidan_threads WHERE id = $1
             UNION ALL
             SELECT t.id, t.parent_thread_id, anc.d + 1
             FROM maidan_threads t JOIN anc ON t.id = anc.parent
         )
         SELECT COALESCE(MAX(d), CAST(0 AS BIGINT)) AS depth FROM anc",
    )
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("depth"))
}

/// Count the `tool_use` blocks recorded across a thread's (non-tombstoned)
/// messages' `content` (Cluster 173) — the tool calls counted against `max_tools`.
pub async fn count_tool_uses(pool: &PgPool, thread_id: ThreadId) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n
         FROM (
             SELECT content FROM maidan_messages
             WHERE thread_id = $1 AND tombstoned_at IS NULL
               AND content IS NOT NULL AND jsonb_typeof(content) = 'array'
         ) m
         CROSS JOIN LATERAL jsonb_array_elements(m.content) AS blk
         WHERE blk->>'type' = 'tool_use'",
    )
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n"))
}
