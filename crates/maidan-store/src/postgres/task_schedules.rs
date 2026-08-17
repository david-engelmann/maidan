use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, MemberId, NewTaskSchedule, TaskSchedule, TaskScheduleId, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str = "id, workspace_id, channel_id, title, interval_secs, next_run_at, last_run_at, active, created_by, created_at, updated_at";

/// Create a task schedule (Cluster 226) — see the SQLite twin. `active`,
/// `last_run_at`, and the timestamps take their column defaults.
pub async fn create(pool: &PgPool, new: NewTaskSchedule) -> Result<TaskSchedule, StoreError> {
    let id = TaskScheduleId::new();
    let row = sqlx::query(
        "INSERT INTO maidan_task_schedules
             (id, workspace_id, channel_id, title, interval_secs, next_run_at, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, workspace_id, channel_id, title, interval_secs, next_run_at, last_run_at, active, created_by, created_at, updated_at",
    )
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(&new.title)
    .bind(new.interval_secs)
    .bind(new.next_run_at)
    .bind(new.created_by.0)
    .fetch_one(pool)
    .await?;
    Ok(row_to_schedule(&row))
}

pub async fn get(pool: &PgPool, id: TaskScheduleId) -> Result<TaskSchedule, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_task_schedules WHERE id = $1"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_schedule(&row))
}

pub async fn list(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<TaskSchedule>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_task_schedules WHERE workspace_id = $1 ORDER BY next_run_at ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_schedule).collect())
}

pub async fn delete(pool: &PgPool, id: TaskScheduleId) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_task_schedules WHERE id = $1")
        .bind(id.0)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn due(
    pool: &PgPool,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<TaskSchedule>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_task_schedules
         WHERE active AND next_run_at <= $1
         ORDER BY next_run_at ASC
         LIMIT $2"
    ))
    .bind(now)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_schedule).collect())
}

/// Atomically claim + advance the oldest due schedule (Cluster 227). `FOR UPDATE
/// SKIP LOCKED` lets concurrent replicas each claim a distinct row. Recurring →
/// `next_run_at = now + interval` (fire-once-per-tick); one-shot → `active =
/// false`. See the SQLite twin.
pub async fn claim_next_due(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Option<TaskSchedule>, StoreError> {
    let row = sqlx::query(
        "WITH due AS (
             SELECT id FROM maidan_task_schedules
             WHERE active AND next_run_at <= $1
             ORDER BY next_run_at ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_task_schedules t
         SET last_run_at = $1,
             updated_at = now(),
             active = (t.interval_secs IS NOT NULL),
             next_run_at = CASE WHEN t.interval_secs IS NULL THEN t.next_run_at
                                ELSE $1 + make_interval(secs => t.interval_secs) END
         FROM due
         WHERE t.id = due.id
         RETURNING t.id, t.workspace_id, t.channel_id, t.title, t.interval_secs, t.next_run_at, t.last_run_at, t.active, t.created_by, t.created_at, t.updated_at",
    )
    .bind(now)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_schedule))
}

/// Pause / resume a schedule (Cluster 228) — see the SQLite twin.
pub async fn set_active(
    pool: &PgPool,
    id: TaskScheduleId,
    active: bool,
) -> Result<TaskSchedule, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_task_schedules SET active = $1, updated_at = now()
         WHERE id = $2
         RETURNING {COLS}"
    ))
    .bind(active)
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_schedule(&row))
}

fn row_to_schedule(row: &sqlx::postgres::PgRow) -> TaskSchedule {
    TaskSchedule {
        id: TaskScheduleId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        title: row.get("title"),
        interval_secs: row.get::<Option<i64>, _>("interval_secs"),
        next_run_at: row.get::<DateTime<Utc>, _>("next_run_at"),
        last_run_at: row.get::<Option<DateTime<Utc>>, _>("last_run_at"),
        active: row.get::<bool, _>("active"),
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}
