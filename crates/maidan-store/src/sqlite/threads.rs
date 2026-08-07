use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, MemberId, NewThread, Thread, ThreadClaimResult, ThreadId, ThreadState, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &SqlitePool, new: NewThread) -> Result<Thread, StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::new_v4();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id",
    )
    .bind(id)
    .bind(new.channel_id.0)
    .bind(new.parent_thread_id.map(|p| p.0))
    .bind(new.title.as_deref())
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .fetch_one(pool)
    .await?;
    row_to_thread(&row)
}

pub async fn get(pool: &SqlitePool, id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id
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
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id
         FROM maidan_threads WHERE channel_id = ? ORDER BY created_at DESC",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// Set the assignee unconditionally (assign / handoff). `NotFound` if absent or
/// tombstoned (Cluster 171).
pub async fn assign(
    pool: &SqlitePool,
    thread_id: ThreadId,
    assignee_id: MemberId,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, updated_at = ?
         WHERE id = ? AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id",
    )
    .bind(assignee_id.0)
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Clear the assignee (Cluster 171). `NotFound` if absent.
pub async fn unassign(pool: &SqlitePool, thread_id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, updated_at = ?
         WHERE id = ?
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Atomic compare-and-set claim (Cluster 171): `assignee_id IS NULL` guards the
/// UPDATE so only one concurrent claimer wins. `None` → already assigned (or
/// absent); disambiguate with a follow-up read.
pub async fn claim(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<ThreadClaimResult, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, updated_at = ?
         WHERE id = ? AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id",
    )
    .bind(member_id.0)
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(row) => Ok(ThreadClaimResult {
            thread: row_to_thread(&row)?,
            claimed: true,
        }),
        None => Ok(ThreadClaimResult {
            thread: get(pool, thread_id).await?,
            claimed: false,
        }),
    }
}

pub async fn list_for_workspace(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ?
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
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    after: Option<ThreadId>,
    limit: i64,
) -> Result<Vec<Thread>, StoreError> {
    let cursor = after.map(|t| t.0);
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ?
           AND t.tombstoned_at IS NULL
           AND (? IS NULL OR (t.created_at, t.id) > (
                 SELECT ct.created_at, ct.id FROM maidan_threads ct WHERE ct.id = ?
               ))
         ORDER BY t.created_at ASC, t.id ASC
         LIMIT ?",
    )
    .bind(workspace_id.0)
    .bind(cursor)
    .bind(cursor)
    .bind(limit.max(0))
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

async fn validate_parent(
    pool: &SqlitePool,
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
    let parent: Option<Uuid> = row.get("parent_thread_id");
    let assignee: Option<Uuid> = row.get("assignee_id");
    Ok(Thread {
        id: ThreadId(row.get::<Uuid, _>("id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        parent_thread_id: parent.map(ThreadId),
        title: row.get("title"),
        state,
        assignee_id: assignee.map(MemberId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    })
}
