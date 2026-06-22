use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, NewThread, Thread, ThreadId, ThreadState, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &PgPool, new: NewThread) -> Result<Thread, StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title)
         VALUES ($1, $2, $3, $4)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.channel_id.0)
    .bind(new.parent_thread_id.map(|p| p.0))
    .bind(new.title.as_deref())
    .fetch_one(pool)
    .await?;
    row_to_thread(&row)
}

pub async fn get(pool: &PgPool, id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at
         FROM maidan_threads WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

pub async fn list(pool: &PgPool, channel_id: ChannelId) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at
         FROM maidan_threads WHERE channel_id = $1 ORDER BY created_at DESC",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

pub async fn list_for_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = $1
         ORDER BY t.created_at DESC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// One keyset page of a workspace's live threads, ordered `(created_at, id)`
/// ascending. `after` is an exclusive cursor (the last thread id of the prior
/// page); `None` starts from the beginning. Filters tombstoned threads in SQL
/// and `LIMIT`s in the DB, so context assembly no longer loads every thread.
pub async fn page_for_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    after: Option<ThreadId>,
    limit: i64,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = $1
           AND t.tombstoned_at IS NULL
           AND ($2::uuid IS NULL OR (t.created_at, t.id) > (
                 SELECT ct.created_at, ct.id FROM maidan_threads ct WHERE ct.id = $2
               ))
         ORDER BY t.created_at ASC, t.id ASC
         LIMIT $3",
    )
    .bind(workspace_id.0)
    .bind(after.map(|t| t.0))
    .bind(limit.max(0))
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

async fn validate_parent(
    pool: &PgPool,
    channel_id: ChannelId,
    parent_thread_id: Option<ThreadId>,
) -> Result<(), StoreError> {
    let Some(parent_id) = parent_thread_id else {
        return Ok(());
    };
    let parent = get(pool, parent_id).await?;
    if parent.channel_id != channel_id {
        return Err(StoreError::InvalidInput(
            "parent thread must be in the same channel".into(),
        ));
    }
    if parent.tombstoned_at.is_some() {
        return Err(StoreError::NotFound);
    }
    if parent.state == ThreadState::Archived {
        return Err(StoreError::Conflict(
            "cannot create child under an archived parent".into(),
        ));
    }
    Ok(())
}

pub(super) fn row_to_thread(row: &sqlx::postgres::PgRow) -> Result<Thread, StoreError> {
    let state_str: String = row.get("state");
    let state = parse_state(&state_str)?;
    let parent: Option<Uuid> = row.get("parent_thread_id");
    Ok(Thread {
        id: ThreadId(row.get::<Uuid, _>("id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        parent_thread_id: parent.map(ThreadId),
        title: row.get("title"),
        state,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    })
}

fn parse_state(state_str: &str) -> Result<ThreadState, StoreError> {
    match state_str {
        "open" => Ok(ThreadState::Open),
        "in_review" => Ok(ThreadState::InReview),
        "closed" => Ok(ThreadState::Closed),
        "archived" => Ok(ThreadState::Archived),
        other => Err(StoreError::InvalidInput(format!(
            "unknown thread state: {other}"
        ))),
    }
}
