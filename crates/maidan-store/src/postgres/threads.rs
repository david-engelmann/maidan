use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, ClaimLeaseId, Event, MemberId, NewThread, QueueDepth, StoredEvent, Thread,
    ThreadClaimResult, ThreadId, ThreadState, WorkspaceId,
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
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
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
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
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
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at
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
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at
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
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = $1, claim_lease_id = $3, work_started_at = NULL, updated_at = NOW()
         WHERE id = $2 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(assignee_id.0)
    .bind(thread_id.0)
    .bind(lease.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Assign a thread and append its `ThreadAssignmentChanged` event in one
/// transaction (Cluster 209). The previous assignee is captured in the same tx.
pub async fn assign_with_event(
    pool: &PgPool,
    thread_id: ThreadId,
    assignee_id: MemberId,
    actor_id: MemberId,
    note: Option<String>,
) -> Result<(Thread, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let previous = sqlx::query(
        "SELECT assignee_id FROM maidan_threads WHERE id = $1 AND tombstoned_at IS NULL",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?
    .get::<Option<Uuid>, _>("assignee_id")
    .map(MemberId);
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = $1, claim_lease_id = $3, work_started_at = NULL, updated_at = NOW()
         WHERE id = $2 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(assignee_id.0)
    .bind(thread_id.0)
    .bind(lease.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let thread = row_to_thread(&row)?;
    let stored = append_assignment_event(&mut tx, &thread, actor_id, previous, note).await?;
    tx.commit().await?;
    Ok((thread, stored))
}

/// Clear the assignee (Cluster 171). `NotFound` if absent.
pub async fn unassign(pool: &PgPool, thread_id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, claim_lease_id = NULL, work_started_at = NULL, updated_at = NOW()
         WHERE id = $1
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Clear the assignee and append its `ThreadAssignmentChanged` event in one
/// transaction (Cluster 209). No handoff note.
pub async fn unassign_with_event(
    pool: &PgPool,
    thread_id: ThreadId,
    actor_id: MemberId,
) -> Result<(Thread, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let previous = sqlx::query("SELECT assignee_id FROM maidan_threads WHERE id = $1")
        .bind(thread_id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?
        .get::<Option<Uuid>, _>("assignee_id")
        .map(MemberId);
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, claim_lease_id = NULL, work_started_at = NULL, updated_at = NOW()
         WHERE id = $1
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let thread = row_to_thread(&row)?;
    let stored = append_assignment_event(&mut tx, &thread, actor_id, previous, None).await?;
    tx.commit().await?;
    Ok((thread, stored))
}

/// Build + append a `ThreadAssignmentChanged` event on a caller-supplied tx
/// (Cluster 209). Shared by the assignment `*_with_event` mutations.
async fn append_assignment_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread: &Thread,
    actor_id: MemberId,
    previous_assignee_id: Option<MemberId>,
    note: Option<String>,
) -> Result<StoredEvent, StoreError> {
    let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, thread.id).await?;
    let event = Event::ThreadAssignmentChanged {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id: thread.id,
        actor_id,
        previous_assignee_id,
        assignee_id: thread.assignee_id,
        note,
        thread: thread.clone(),
    };
    events::append_in_tx(tx, &event).await
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
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = $1, claim_lease_id = $3, work_started_at = NULL, updated_at = NOW()
         WHERE id = $2 AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(member_id.0)
    .bind(thread_id.0)
    .bind(lease.0)
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

/// Atomic claim + its `ThreadAssignmentChanged` event in one tx (Cluster 209).
/// Conditional: the event is appended **only** when the CAS actually claimed.
/// `previous_assignee_id` is `None` (plain claim guards on unassigned).
pub async fn claim_with_event(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<(ThreadClaimResult, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = $1, claim_lease_id = $3, work_started_at = NULL, updated_at = NOW()
         WHERE id = $2 AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(member_id.0)
    .bind(thread_id.0)
    .bind(lease.0)
    .fetch_optional(&mut *tx)
    .await?;
    match row {
        Some(row) => {
            let thread = row_to_thread(&row)?;
            let stored = append_assignment_event(&mut tx, &thread, member_id, None, None).await?;
            tx.commit().await?;
            Ok((
                ThreadClaimResult {
                    thread,
                    claimed: true,
                },
                Some(stored),
            ))
        }
        None => {
            tx.commit().await?;
            let thread = get(pool, thread_id).await?;
            Ok((
                ThreadClaimResult {
                    thread,
                    claimed: false,
                },
                None,
            ))
        }
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
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at
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
    let lease = ClaimLeaseId::new();
    // Claimable = unassigned OR the lease has expired (dead-agent recovery;
    // Cluster 192). FOR UPDATE SKIP LOCKED keeps concurrent claimers distinct.
    let row = sqlx::query(
        "WITH next AS (
             SELECT c.id FROM maidan_threads c
             WHERE c.channel_id = $2 AND c.tombstoned_at IS NULL
               AND (c.assignee_id IS NULL OR (c.assignment_expires_at IS NOT NULL AND c.assignment_expires_at < NOW()))
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_dependencies d
                   JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                   WHERE d.thread_id = c.id AND dep.state NOT IN ('closed', 'archived')
               )
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_required_skills trs
                   WHERE trs.thread_id = c.id
                     AND NOT EXISTS (
                         SELECT 1 FROM maidan_member_skills ms
                         WHERE ms.member_id = $1 AND ms.skill = trs.skill
                     )
               )
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_approval_gates g
                   WHERE g.thread_id = c.id AND g.state = 'pending'
               )
             ORDER BY c.created_at ASC, c.id ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_threads t SET assignee_id = $1, assignment_expires_at = $3, claim_lease_id = $4, work_started_at = NULL, updated_at = NOW()
         FROM next WHERE t.id = next.id
         RETURNING t.id, t.channel_id, t.parent_thread_id, t.title, t.state, t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at",
    )
    .bind(member_id.0)
    .bind(channel_id.0)
    .bind(expires)
    .bind(lease.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_thread).transpose()
}

/// Task-queue depth for a channel (Cluster 224) — see the SQLite twin. Uses
/// `NOW()` inline (as `claim_next` does) so `ready` matches its claimability
/// predicate exactly.
pub async fn channel_queue_depth(
    pool: &PgPool,
    channel_id: ChannelId,
) -> Result<QueueDepth, StoreError> {
    let row = sqlx::query(
        "SELECT
             COUNT(*) AS open_count,
             COALESCE(SUM(CASE WHEN t.assignee_id IS NOT NULL
                       AND (t.assignment_expires_at IS NULL OR t.assignment_expires_at >= NOW())
                     THEN 1 ELSE 0 END), 0) AS assigned_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < NOW()))
                       AND NOT EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                     THEN 1 ELSE 0 END), 0) AS ready_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < NOW()))
                       AND EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                     THEN 1 ELSE 0 END), 0) AS blocked_count
         FROM maidan_threads t
         WHERE t.channel_id = $1
           AND t.state NOT IN ('closed', 'archived')
           AND t.tombstoned_at IS NULL",
    )
    .bind(channel_id.0)
    .fetch_one(pool)
    .await?;
    Ok(QueueDepth {
        open: row.get::<i64, _>("open_count"),
        ready: row.get::<i64, _>("ready_count"),
        assigned: row.get::<i64, _>("assigned_count"),
        blocked: row.get::<i64, _>("blocked_count"),
    })
}

/// Atomic claim-next + its `ThreadAssignmentChanged` event in one tx (Cluster
/// 209). Conditional: the event is appended **only** when a thread was claimed;
/// an empty channel yields `(None, None)`. `previous_assignee_id` is `None`
/// (behaviour-preserving — matches the old route).
pub async fn claim_next_with_event(
    pool: &PgPool,
    channel_id: ChannelId,
    member_id: MemberId,
    lease_secs: Option<i64>,
) -> Result<(Option<Thread>, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let expires = lease_secs.map(|s| chrono::Utc::now() + chrono::Duration::seconds(s));
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "WITH next AS (
             SELECT c.id FROM maidan_threads c
             WHERE c.channel_id = $2 AND c.tombstoned_at IS NULL
               AND (c.assignee_id IS NULL OR (c.assignment_expires_at IS NOT NULL AND c.assignment_expires_at < NOW()))
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_dependencies d
                   JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                   WHERE d.thread_id = c.id AND dep.state NOT IN ('closed', 'archived')
               )
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_required_skills trs
                   WHERE trs.thread_id = c.id
                     AND NOT EXISTS (
                         SELECT 1 FROM maidan_member_skills ms
                         WHERE ms.member_id = $1 AND ms.skill = trs.skill
                     )
               )
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_approval_gates g
                   WHERE g.thread_id = c.id AND g.state = 'pending'
               )
             ORDER BY c.created_at ASC, c.id ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_threads t SET assignee_id = $1, assignment_expires_at = $3, claim_lease_id = $4, work_started_at = NULL, updated_at = NOW()
         FROM next WHERE t.id = next.id
         RETURNING t.id, t.channel_id, t.parent_thread_id, t.title, t.state, t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at",
    )
    .bind(member_id.0)
    .bind(channel_id.0)
    .bind(expires)
    .bind(lease.0)
    .fetch_optional(&mut *tx)
    .await?;
    match row {
        Some(row) => {
            let thread = row_to_thread(&row)?;
            let stored = append_assignment_event(&mut tx, &thread, member_id, None, None).await?;
            tx.commit().await?;
            Ok((Some(thread), Some(stored)))
        }
        None => {
            tx.commit().await?;
            Ok((None, None))
        }
    }
}

/// Extend a claim's lease (heartbeat), only for the current assignee (Cluster
/// 192). `NotFound` if the thread is gone or the caller isn't the holder.
pub async fn renew_claim(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
    lease_secs: i64,
) -> Result<Thread, StoreError> {
    let expires = chrono::Utc::now() + chrono::Duration::seconds(lease_secs);
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignment_expires_at = $1, updated_at = NOW()
         WHERE id = $2 AND assignee_id = $3 AND claim_lease_id = $4 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(expires)
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Stamp the working clock (Cluster 351): the current holder acknowledges the
/// claim and begins work. Fenced by `(assignee_id, claim_lease_id)` — only the
/// live holder presenting the matching token can start the clock. `COALESCE`
/// keeps the first start time, so a re-acknowledge within the same claim epoch
/// is idempotent (a reclaim reset `work_started_at` to NULL, so the next holder
/// stamps fresh). `NotFound` if the thread is gone, the caller isn't the holder,
/// or the token is stale.
pub async fn acknowledge_claim(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET work_started_at = COALESCE(work_started_at, NOW()), updated_at = NOW()
         WHERE id = $1 AND assignee_id = $2 AND claim_lease_id = $3 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
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
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at
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
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at
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

/// One keyset page of a channel's **live** threads, ordered `(created_at, id)`
/// ascending (Cluster 343). `after` is an exclusive cursor (the prior page's last
/// thread id); `None` starts from the beginning. The channel-scoped twin of
/// [`page_for_workspace`] — bounds the previously-unbounded channel thread list.
pub async fn page_for_channel(
    pool: &PgPool,
    channel_id: ChannelId,
    after: Option<ThreadId>,
    limit: i64,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at
         FROM maidan_threads t
         WHERE t.channel_id = $1
           AND t.tombstoned_at IS NULL
           AND ($2::uuid IS NULL OR (t.created_at, t.id) > (
                 SELECT ct.created_at, ct.id FROM maidan_threads ct WHERE ct.id = $2
               ))
         ORDER BY t.created_at ASC, t.id ASC
         LIMIT $3",
    )
    .bind(channel_id.0)
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
        claim_lease_id: row
            .get::<Option<Uuid>, _>("claim_lease_id")
            .map(ClaimLeaseId),
        work_started_at: row.get::<Option<DateTime<Utc>>, _>("work_started_at"),
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
