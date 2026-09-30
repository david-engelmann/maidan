use chrono::{DateTime, SecondsFormat, Utc};
use maidan_types::{ThreadId, WorkspaceId, DM_CHANNEL_NAME};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::a2a::{A2aPushConfigRow, A2aTaskQuery, A2aTaskRow, A2aTaskWrite};
use crate::error::StoreError;
use crate::thread_access::readable_row;

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

/// The `updated_at` text form: always millisecond `...Z`, so rows and cursors
/// compare correctly as strings.
fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| StoreError::InvalidInput(format!("bad timestamp: {e}")))
}

type TaskRow = (String, Uuid, Option<Uuid>, String, String);

fn task_row(
    (id, workspace_id, thread_id, updated_at, json): TaskRow,
) -> Result<A2aTaskRow, StoreError> {
    Ok(A2aTaskRow {
        id,
        workspace_id: WorkspaceId(workspace_id),
        thread_id: thread_id.map(ThreadId),
        updated_at: parse_ts(&updated_at)?,
        task_json: serde_json::from_str(&json)?,
    })
}

pub async fn upsert_task(pool: &SqlitePool, task: A2aTaskWrite<'_>) -> Result<(), StoreError> {
    let json = serde_json::to_string(&task.task_json)?;
    sqlx::query(
        "INSERT INTO maidan_a2a_tasks
            (id, workspace_id, context_id, thread_id, state, task_json, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            context_id = excluded.context_id,
            thread_id = excluded.thread_id,
            state = excluded.state,
            task_json = excluded.task_json,
            updated_at = excluded.updated_at",
    )
    .bind(task.task_id)
    .bind(task.workspace_id.0)
    .bind(task.context_id)
    .bind(task.thread_id.map(|t| t.0))
    .bind(task.state)
    .bind(json)
    .bind(ts(task.status_at))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_task(pool: &SqlitePool, task_id: &str) -> Result<Option<A2aTaskRow>, StoreError> {
    let row: Option<TaskRow> = sqlx::query_as(
        "SELECT id, workspace_id, thread_id, updated_at, task_json
         FROM maidan_a2a_tasks WHERE id = ?",
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    row.map(task_row).transpose()
}

/// The filters `list_tasks` and `count_tasks` share: `?1` workspace, `?2`
/// context, `?3` state, `?4` updated since, `?5` reader, `?6` the DM channel
/// name.
fn task_filters() -> String {
    let readable = readable_row("maidan_a2a_tasks.thread_id", "?1", "?5", "?6");
    format!(
        "workspace_id = ?1
           AND (?2 IS NULL OR context_id = ?2)
           AND (?3 IS NULL OR state = ?3)
           AND (?4 IS NULL OR updated_at >= ?4)
           AND {readable}"
    )
}

pub async fn list_tasks(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    query: A2aTaskQuery<'_>,
) -> Result<Vec<A2aTaskRow>, StoreError> {
    let (before_at, before_id) = match query.before {
        Some((at, id)) => (Some(ts(at)), Some(id)),
        None => (None, None),
    };
    let rows: Vec<TaskRow> = sqlx::query_as(&format!(
        "SELECT id, workspace_id, thread_id, updated_at, task_json
         FROM maidan_a2a_tasks
         WHERE {}
           AND (?7 IS NULL OR updated_at < ?7 OR (updated_at = ?7 AND id < ?8))
         ORDER BY updated_at DESC, id DESC LIMIT ?9",
        task_filters()
    ))
    .bind(workspace_id.0)
    .bind(query.context_id)
    .bind(query.state)
    .bind(query.updated_since.map(ts))
    .bind(query.readable_by.map(|m| m.0))
    .bind(DM_CHANNEL_NAME)
    .bind(before_at)
    .bind(before_id)
    .bind(query.limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(task_row).collect()
}

pub async fn count_tasks(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    query: A2aTaskQuery<'_>,
) -> Result<i64, StoreError> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM maidan_a2a_tasks WHERE {}",
        task_filters()
    ))
    .bind(workspace_id.0)
    .bind(query.context_id)
    .bind(query.state)
    .bind(query.updated_since.map(ts))
    .bind(query.readable_by.map(|m| m.0))
    .bind(DM_CHANNEL_NAME)
    .fetch_one(pool)
    .await?)
}

pub async fn get_context_thread(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    context_id: &str,
) -> Result<Option<ThreadId>, StoreError> {
    let row: Option<Uuid> = sqlx::query_scalar(
        "SELECT thread_id FROM maidan_a2a_contexts WHERE workspace_id = ? AND context_id = ?",
    )
    .bind(workspace_id.0)
    .bind(context_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(ThreadId))
}

pub async fn bind_context(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    context_id: &str,
    thread_id: ThreadId,
) -> Result<ThreadId, StoreError> {
    let bound: Uuid = sqlx::query_scalar(
        "INSERT INTO maidan_a2a_contexts (workspace_id, context_id, thread_id)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(workspace_id, context_id) DO UPDATE SET context_id = excluded.context_id
         RETURNING thread_id",
    )
    .bind(workspace_id.0)
    .bind(context_id)
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?;
    Ok(ThreadId(bound))
}

type PushRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

const PUSH_COLS: &str = "task_id, config_id, push_url, token_ciphertext, auth_scheme, \
                         auth_credentials_ciphertext";

fn push_row(
    (task_id, config_id, url, token_ciphertext, auth_scheme, auth_credentials_ciphertext): PushRow,
) -> A2aPushConfigRow {
    A2aPushConfigRow {
        task_id,
        config_id,
        url,
        token_ciphertext,
        auth_scheme,
        auth_credentials_ciphertext,
    }
}

pub async fn upsert_task_push_config(
    pool: &SqlitePool,
    config: &A2aPushConfigRow,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_a2a_task_push_configs
            (task_id, config_id, push_url, token_ciphertext, auth_scheme, auth_credentials_ciphertext)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(task_id, config_id) DO UPDATE SET
            push_url = excluded.push_url,
            token_ciphertext = excluded.token_ciphertext,
            auth_scheme = excluded.auth_scheme,
            auth_credentials_ciphertext = excluded.auth_credentials_ciphertext",
    )
    .bind(&config.task_id)
    .bind(&config.config_id)
    .bind(&config.url)
    .bind(&config.token_ciphertext)
    .bind(&config.auth_scheme)
    .bind(&config.auth_credentials_ciphertext)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_task_push_config(
    pool: &SqlitePool,
    task_id: &str,
    config_id: &str,
) -> Result<Option<A2aPushConfigRow>, StoreError> {
    let row: Option<PushRow> = sqlx::query_as(&format!(
        "SELECT {PUSH_COLS} FROM maidan_a2a_task_push_configs WHERE task_id = ? AND config_id = ?"
    ))
    .bind(task_id)
    .bind(config_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(push_row))
}

pub async fn list_task_push_configs(
    pool: &SqlitePool,
    task_id: &str,
) -> Result<Vec<A2aPushConfigRow>, StoreError> {
    let rows: Vec<PushRow> = sqlx::query_as(&format!(
        "SELECT {PUSH_COLS} FROM maidan_a2a_task_push_configs
         WHERE task_id = ? ORDER BY created_at ASC, config_id ASC"
    ))
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(push_row).collect())
}

pub async fn page_task_push_configs(
    pool: &SqlitePool,
    task_id: &str,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<A2aPushConfigRow>, StoreError> {
    let rows: Vec<PushRow> = sqlx::query_as(&format!(
        "SELECT {PUSH_COLS} FROM maidan_a2a_task_push_configs
         WHERE task_id = ?1 AND (?2 IS NULL OR config_id > ?2)
         ORDER BY config_id ASC LIMIT ?3"
    ))
    .bind(task_id)
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(push_row).collect())
}

pub async fn delete_task_push_config(
    pool: &SqlitePool,
    task_id: &str,
    config_id: &str,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_a2a_task_push_configs WHERE task_id = ? AND config_id = ?")
            .bind(task_id)
            .bind(config_id)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}
