use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, MemberId, NewTaskSchedule, TaskSchedule, TaskScheduleId, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str = "id, workspace_id, channel_id, title, interval_secs, next_run_at, last_run_at, active, created_by, created_at, updated_at";

/// Create a task schedule (Cluster 226). Starts active; `last_run_at` is NULL
/// until the sweeper first fires it.
pub async fn create(pool: &SqlitePool, new: NewTaskSchedule) -> Result<TaskSchedule, StoreError> {
    let id = TaskScheduleId::new();
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_task_schedules
             (id, workspace_id, channel_id, title, interval_secs, next_run_at, last_run_at, active, created_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, NULL, 1, ?, ?, ?)
         RETURNING id, workspace_id, channel_id, title, interval_secs, next_run_at, last_run_at, active, created_by, created_at, updated_at",
    )
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(&new.title)
    .bind(new.interval_secs)
    .bind(new.next_run_at.to_rfc3339())
    .bind(new.created_by.0)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_schedule(&row))
}

pub async fn get(pool: &SqlitePool, id: TaskScheduleId) -> Result<TaskSchedule, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_task_schedules WHERE id = ?"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_schedule(&row))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<TaskSchedule>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_task_schedules WHERE workspace_id = ? ORDER BY next_run_at ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_schedule).collect())
}

/// Delete a schedule; `true` when a row was removed.
pub async fn delete(pool: &SqlitePool, id: TaskScheduleId) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_task_schedules WHERE id = ?")
        .bind(id.0)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Active schedules whose `next_run_at` has arrived, oldest first (Cluster 226).
/// The sweeper's due-scan; `limit` bounds a batch.
pub async fn due(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<TaskSchedule>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_task_schedules
         WHERE active = 1 AND next_run_at <= ?
         ORDER BY next_run_at ASC
         LIMIT ?"
    ))
    .bind(now.to_rfc3339())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_schedule).collect())
}

/// Atomically claim + advance the oldest due schedule (Cluster 227). SQLite
/// serializes writers, so selecting the candidate then updating it inside one
/// transaction cannot double-claim. Recurring → `next_run_at = now + interval`
/// (fire-once-per-tick); one-shot → `active = 0`.
pub async fn claim_next_due(
    pool: &SqlitePool,
    now: DateTime<Utc>,
) -> Result<Option<TaskSchedule>, StoreError> {
    let now_s = now.to_rfc3339();
    let mut tx = pool.begin().await?;
    let candidate = sqlx::query(
        "SELECT id, interval_secs, next_run_at FROM maidan_task_schedules
         WHERE active = 1 AND next_run_at <= ?
         ORDER BY next_run_at ASC
         LIMIT 1",
    )
    .bind(&now_s)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(cand) = candidate else {
        return Ok(None);
    };
    let id: Uuid = cand.get("id");
    let interval_secs: Option<i64> = cand.get("interval_secs");
    let (next_run_at, active) = match interval_secs {
        Some(secs) => ((now + chrono::Duration::seconds(secs)).to_rfc3339(), true),
        None => (
            cand.get::<DateTime<Utc>, _>("next_run_at").to_rfc3339(),
            false,
        ),
    };
    let row = sqlx::query(&format!(
        "UPDATE maidan_task_schedules
         SET next_run_at = ?, last_run_at = ?, active = ?, updated_at = ?
         WHERE id = ?
         RETURNING {COLS}"
    ))
    .bind(&next_run_at)
    .bind(&now_s)
    .bind(active)
    .bind(&now_s)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(row_to_schedule(&row)))
}

/// Pause / resume a schedule (Cluster 228).
pub async fn set_active(
    pool: &SqlitePool,
    id: TaskScheduleId,
    active: bool,
) -> Result<TaskSchedule, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_task_schedules SET active = ?, updated_at = ?
         WHERE id = ?
         RETURNING {COLS}"
    ))
    .bind(active)
    .bind(Utc::now().to_rfc3339())
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_schedule(&row))
}

fn row_to_schedule(row: &sqlx::sqlite::SqliteRow) -> TaskSchedule {
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
