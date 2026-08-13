use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, Event, MemberId, NewThread, StoredEvent, Thread, ThreadClaimResult, ThreadId,
    ThreadState, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::postgres::events;

pub async fn create(pool: &PgPool, new: NewThread) -> Result<Thread, StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title)
         VALUES ($1, $2, $3, $4)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at",
    )
    .bind(id)
    .bind(new.channel_id.0)
    .bind(new.parent_thread_id.map(|p| p.0))
    .bind(new.title.as_deref())
    .fetch_one(pool)
    .await?;
    row_to_thread(&row)
}

/// Insert a thread and append its `ThreadCreated` event in one transaction
/// (Cluster 205 transactional outbox) — see the SQLite twin.
pub async fn create_with_event(
    pool: &PgPool,
    new: NewThread,
) -> Result<(Thread, StoredEvent), StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::new_v4();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title)
         VALUES ($1, $2, $3, $4)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at",
    )
    .bind(id)
    .bind(new.channel_id.0)
    .bind(new.parent_thread_id.map(|p| p.0))
    .bind(new.title.as_deref())
    .fetch_one(&mut *tx)
    .await?;
    let thread = row_to_thread(&row)?;
    let workspace_id: Uuid =
        sqlx::query_scalar("SELECT workspace_id FROM maidan_channels WHERE id = $1")
            .bind(new.channel_id.0)
            .fetch_one(&mut *tx)
            .await?;
    let event = Event::ThreadCreated {
        occurred_at: Utc::now(),
        workspace_id: WorkspaceId(workspace_id),
        channel_id: new.channel_id,
        thread: thread.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((thread, stored))
}

pub async fn get(pool: &PgPool, id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at
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
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at
         FROM maidan_threads WHERE channel_id = $1 ORDER BY created_at DESC",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// Set the assignee unconditionally (assign / handoff). `NotFound` if absent or
/// tombstoned — claiming dead work is a bug (Cluster 171).
pub async fn assign(
    pool: &PgPool,
    thread_id: ThreadId,
    assignee_id: MemberId,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = $1, updated_at = NOW()
         WHERE id = $2 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at",
    )
    .bind(assignee_id.0)
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Clear the assignee (Cluster 171). `NotFound` if absent.
pub async fn unassign(pool: &PgPool, thread_id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, updated_at = NOW()
         WHERE id = $1
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Atomic compare-and-set claim (Cluster 171): the `assignee_id IS NULL`
/// predicate + row lock guarantees only one concurrent claimer wins. A `None`
/// result means the row was already assigned (or absent) — disambiguate with a
/// follow-up read.
pub async fn claim(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<ThreadClaimResult, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = $1, updated_at = NOW()
         WHERE id = $2 AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at",
    )
    .bind(member_id.0)
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

/// Threads in `workspace_id` currently assigned to `member_id` — an agent's work
/// queue (Cluster 190). Live threads only, oldest first. Uses `idx_threads_assignee`.
pub async fn list_assigned(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = $1 AND t.assignee_id = $2 AND t.tombstoned_at IS NULL
         ORDER BY t.created_at ASC, t.id ASC",
    )
    .bind(workspace_id.0)
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// Atomically claim the oldest unassigned live thread in `channel_id` for
/// `member_id` (Cluster 190). `FOR UPDATE SKIP LOCKED` is the canonical
/// concurrent work-queue pattern: parallel claimers skip each other's locked
/// candidate and each gets a distinct thread. `None` when there is no unassigned
/// work.
pub async fn claim_next(
    pool: &PgPool,
    channel_id: ChannelId,
    member_id: MemberId,
    lease_secs: Option<i64>,
) -> Result<Option<Thread>, StoreError> {
    let expires = lease_secs.map(|s| chrono::Utc::now() + chrono::Duration::seconds(s));
    // Claimable = unassigned OR the lease has expired (dead-agent recovery;
    // Cluster 192). FOR UPDATE SKIP LOCKED keeps concurrent claimers distinct.
    let row = sqlx::query(
        "WITH next AS (
             SELECT id FROM maidan_threads
             WHERE channel_id = $2 AND tombstoned_at IS NULL
               AND (assignee_id IS NULL OR (assignment_expires_at IS NOT NULL AND assignment_expires_at < NOW()))
             ORDER BY created_at ASC, id ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_threads t SET assignee_id = $1, assignment_expires_at = $3, updated_at = NOW()
         FROM next WHERE t.id = next.id
         RETURNING t.id, t.channel_id, t.parent_thread_id, t.title, t.state, t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at",
    )
    .bind(member_id.0)
    .bind(channel_id.0)
    .bind(expires)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_thread).transpose()
}

/// Extend a claim's lease (heartbeat), only for the current assignee (Cluster
/// 192). `NotFound` if the thread is gone or the caller isn't the holder.
pub async fn renew_claim(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_secs: i64,
) -> Result<Thread, StoreError> {
    let expires = chrono::Utc::now() + chrono::Duration::seconds(lease_secs);
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignment_expires_at = $1, updated_at = NOW()
         WHERE id = $2 AND assignee_id = $3 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at",
    )
    .bind(expires)
    .bind(thread_id.0)
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

pub async fn list_for_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at
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
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at
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
    let assignee: Option<Uuid> = row.get("assignee_id");
    Ok(Thread {
        id: ThreadId(row.get::<Uuid, _>("id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        parent_thread_id: parent.map(ThreadId),
        title: row.get("title"),
        state,
        assignee_id: assignee.map(MemberId),
        assignment_expires_at: row.get::<Option<DateTime<Utc>>, _>("assignment_expires_at"),
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
