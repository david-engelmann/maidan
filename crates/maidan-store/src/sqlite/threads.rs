use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, NewThread, Thread, ThreadId, ThreadState};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &SqlitePool, new: NewThread) -> Result<Thread, StoreError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, title, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)
         RETURNING id, channel_id, title, state, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.channel_id.0)
    .bind(new.title.as_deref())
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    row_to_thread(&row)
}

pub async fn get(pool: &SqlitePool, id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "SELECT id, channel_id, title, state, created_at, updated_at, tombstoned_at
         FROM maidan_threads WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

pub async fn list(pool: &SqlitePool, channel_id: ChannelId) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, channel_id, title, state, created_at, updated_at, tombstoned_at
         FROM maidan_threads WHERE channel_id = ? ORDER BY created_at DESC",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

pub(super) fn row_to_thread(row: &sqlx::sqlite::SqliteRow) -> Result<Thread, StoreError> {
    let state_str: String = row.get("state");
    let state = match state_str.as_str() {
        "open" => ThreadState::Open,
        "in_review" => ThreadState::InReview,
        "closed" => ThreadState::Closed,
        "archived" => ThreadState::Archived,
        other => {
            return Err(StoreError::InvalidInput(format!(
                "unknown thread state: {other}"
            )));
        }
    };
    Ok(Thread {
        id: ThreadId(row.get::<Uuid, _>("id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        title: row.get("title"),
        state,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    })
}
