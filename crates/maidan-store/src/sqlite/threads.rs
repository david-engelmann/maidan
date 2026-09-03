use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, ChannelOccupancy, ClaimLeaseId, Event, MemberId, NewThread, QueueDepth, StoredEvent,
    Thread, ThreadClaimResult, ThreadId, ThreadState, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::sqlite::events;

use crate::error::StoreError;

pub async fn create(pool: &SqlitePool, new: NewThread) -> Result<Thread, StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::new_v4();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
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

/// Insert a thread and append its `ThreadCreated` event in one transaction
/// (Cluster 205 transactional outbox). The workspace is resolved in the same tx.
pub async fn create_with_event(
    pool: &SqlitePool,
    new: NewThread,
) -> Result<(Thread, StoredEvent), StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::new_v4();
    let now = Utc::now();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(id)
    .bind(new.channel_id.0)
    .bind(new.parent_thread_id.map(|p| p.0))
    .bind(new.title.as_deref())
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .fetch_one(&mut *tx)
    .await?;
    let thread = row_to_thread(&row)?;
    let workspace_id: Uuid =
        sqlx::query_scalar("SELECT workspace_id FROM maidan_channels WHERE id = ?")
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

pub async fn get(pool: &SqlitePool, id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at
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
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at
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
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, claim_lease_id = ?, work_started_at = NULL, updated_at = ?
         WHERE id = ? AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(assignee_id.0)
    .bind(lease.0)
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Assign a thread and append its `ThreadAssignmentChanged` event in one
/// transaction (Cluster 209). The previous assignee is captured in the same tx
/// (a consistent read, not a separate `get_thread` + race window).
pub async fn assign_with_event(
    pool: &SqlitePool,
    thread_id: ThreadId,
    assignee_id: MemberId,
    actor_id: MemberId,
    note: Option<String>,
) -> Result<(Thread, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let previous = sqlx::query(
        "SELECT assignee_id FROM maidan_threads WHERE id = ? AND tombstoned_at IS NULL",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?
    .get::<Option<Uuid>, _>("assignee_id")
    .map(MemberId);
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, claim_lease_id = ?, work_started_at = NULL, updated_at = ?
         WHERE id = ? AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(assignee_id.0)
    .bind(lease.0)
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let thread = row_to_thread(&row)?;
    let stored = append_assignment_event(&mut tx, &thread, actor_id, previous, note).await?;
    tx.commit().await?;
    Ok((thread, stored))
}

/// Clear the assignee (Cluster 171). `NotFound` if absent.
pub async fn unassign(pool: &SqlitePool, thread_id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, claim_lease_id = NULL, work_started_at = NULL, updated_at = ?
         WHERE id = ?
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Clear the assignee and append its `ThreadAssignmentChanged` event in one
/// transaction (Cluster 209). No handoff note (unassign carries none).
pub async fn unassign_with_event(
    pool: &SqlitePool,
    thread_id: ThreadId,
    actor_id: MemberId,
) -> Result<(Thread, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let previous = sqlx::query("SELECT assignee_id FROM maidan_threads WHERE id = ?")
        .bind(thread_id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?
        .get::<Option<Uuid>, _>("assignee_id")
        .map(MemberId);
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, claim_lease_id = NULL, work_started_at = NULL, updated_at = ?
         WHERE id = ?
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(Utc::now().to_rfc3339())
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
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
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

/// Atomic compare-and-set claim (Cluster 171): `assignee_id IS NULL` guards the
/// UPDATE so only one concurrent claimer wins. `None` → already assigned (or
/// absent); disambiguate with a follow-up read.
pub async fn claim(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<ThreadClaimResult, StoreError> {
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, claim_lease_id = ?, work_started_at = NULL, updated_at = ?
         WHERE id = ? AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(member_id.0)
    .bind(lease.0)
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

/// Atomic claim + its `ThreadAssignmentChanged` event in one tx (Cluster 209).
/// Conditional: the event is appended **only** when the CAS actually claimed
/// (`(result, Some)`); an already-assigned thread yields `(result{claimed:false},
/// None)` — no event. `previous_assignee_id` is `None` (plain claim guards on
/// unassigned).
pub async fn claim_with_event(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<(ThreadClaimResult, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, claim_lease_id = ?, work_started_at = NULL, updated_at = ?
         WHERE id = ? AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(member_id.0)
    .bind(lease.0)
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
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
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ? AND t.assignee_id = ? AND t.tombstoned_at IS NULL
         ORDER BY t.created_at ASC, t.id ASC",
    )
    .bind(workspace_id.0)
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// Atomically claim the oldest unassigned live thread in `channel_id` for
/// `member_id` (Cluster 190) — the "pull the next task" primitive. `None` when
/// the channel has no unassigned work. SQLite serializes writers, so the
/// subquery-guarded UPDATE can't double-assign.
pub async fn claim_next(
    pool: &SqlitePool,
    channel_id: ChannelId,
    member_id: MemberId,
    lease_secs: Option<i64>,
) -> Result<Option<Thread>, StoreError> {
    let now = Utc::now();
    let expires = lease_secs.map(|s| (now + chrono::Duration::seconds(s)).to_rfc3339());
    let lease = ClaimLeaseId::new();
    // Claimable = unassigned OR the current lease has expired (dead-agent
    // recovery; Cluster 192). SQLite serializes writers so this is race-free.
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, assignment_expires_at = ?, claim_lease_id = ?, work_started_at = NULL, updated_at = ?
         WHERE id = (
             SELECT t.id FROM maidan_threads t
             WHERE t.channel_id = ? AND t.tombstoned_at IS NULL
               AND (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < ?))
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_dependencies d
                   JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                   WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived')
               )
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_required_skills trs
                   WHERE trs.thread_id = t.id
                     AND NOT EXISTS (
                         SELECT 1 FROM maidan_member_skills ms
                         WHERE ms.member_id = ? AND ms.skill = trs.skill
                     )
               )
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_approval_gates g
                   WHERE g.thread_id = t.id AND g.state = 'pending'
               )
             ORDER BY t.created_at ASC, t.id ASC
             LIMIT 1
         )
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(member_id.0)
    .bind(&expires)
    .bind(lease.0)
    .bind(now.to_rfc3339())
    .bind(channel_id.0)
    .bind(now.to_rfc3339())
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_thread).transpose()
}

/// Task-queue depth for a channel (Cluster 224). `not_live` (unassigned or lease
/// expired) and the deps `NOT EXISTS` mirror the `claim_next` claimability
/// predicate exactly, so `ready` here is precisely what `claim_next` would take.
pub async fn channel_queue_depth(
    pool: &SqlitePool,
    channel_id: ChannelId,
) -> Result<QueueDepth, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "SELECT
             COUNT(*) AS open_count,
             COALESCE(SUM(CASE WHEN t.assignee_id IS NOT NULL
                       AND (t.assignment_expires_at IS NULL OR t.assignment_expires_at >= ?)
                     THEN 1 ELSE 0 END), 0) AS assigned_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < ?))
                       AND NOT EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                     THEN 1 ELSE 0 END), 0) AS ready_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < ?))
                       AND EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                     THEN 1 ELSE 0 END), 0) AS blocked_count
         FROM maidan_threads t
         WHERE t.channel_id = ?
           AND t.state NOT IN ('closed', 'archived')
           AND t.tombstoned_at IS NULL",
    )
    .bind(&now)
    .bind(&now)
    .bind(&now)
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

/// Channel occupancy (Cluster 351) — the two-clocks refinement of
/// `channel_queue_depth`, splitting the held threads by the working clock. See
/// the Postgres twin. The four sub-counts partition `open`.
pub async fn channel_occupancy(
    pool: &SqlitePool,
    channel_id: ChannelId,
) -> Result<ChannelOccupancy, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "SELECT
             COUNT(*) AS open_count,
             COALESCE(SUM(CASE WHEN t.assignee_id IS NOT NULL
                       AND (t.assignment_expires_at IS NULL OR t.assignment_expires_at >= ?)
                       AND t.work_started_at IS NULL
                     THEN 1 ELSE 0 END), 0) AS claimed_count,
             COALESCE(SUM(CASE WHEN t.assignee_id IS NOT NULL
                       AND (t.assignment_expires_at IS NULL OR t.assignment_expires_at >= ?)
                       AND t.work_started_at IS NOT NULL
                     THEN 1 ELSE 0 END), 0) AS working_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < ?))
                       AND NOT EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                     THEN 1 ELSE 0 END), 0) AS queued_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < ?))
                       AND EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                     THEN 1 ELSE 0 END), 0) AS blocked_count
         FROM maidan_threads t
         WHERE t.channel_id = ?
           AND t.state NOT IN ('closed', 'archived')
           AND t.tombstoned_at IS NULL",
    )
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .bind(channel_id.0)
    .fetch_one(pool)
    .await?;
    Ok(ChannelOccupancy {
        open: row.get::<i64, _>("open_count"),
        queued: row.get::<i64, _>("queued_count"),
        claimed: row.get::<i64, _>("claimed_count"),
        working: row.get::<i64, _>("working_count"),
        blocked: row.get::<i64, _>("blocked_count"),
    })
}

/// Atomic claim-next + its `ThreadAssignmentChanged` event in one tx (Cluster
/// 209). Conditional: the event is appended **only** when a thread was claimed
/// (`(Some(thread), Some(event))`); an empty channel yields `(None, None)`.
/// `previous_assignee_id` is `None` (behaviour-preserving — matches the old
/// route; a lease-expiry reclaim does not surface the prior holder).
pub async fn claim_next_with_event(
    pool: &SqlitePool,
    channel_id: ChannelId,
    member_id: MemberId,
    lease_secs: Option<i64>,
) -> Result<(Option<Thread>, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();
    let expires = lease_secs.map(|s| (now + chrono::Duration::seconds(s)).to_rfc3339());
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, assignment_expires_at = ?, claim_lease_id = ?, work_started_at = NULL, updated_at = ?
         WHERE id = (
             SELECT t.id FROM maidan_threads t
             WHERE t.channel_id = ? AND t.tombstoned_at IS NULL
               AND (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < ?))
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_dependencies d
                   JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                   WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived')
               )
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_required_skills trs
                   WHERE trs.thread_id = t.id
                     AND NOT EXISTS (
                         SELECT 1 FROM maidan_member_skills ms
                         WHERE ms.member_id = ? AND ms.skill = trs.skill
                     )
               )
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_approval_gates g
                   WHERE g.thread_id = t.id AND g.state = 'pending'
               )
             ORDER BY t.created_at ASC, t.id ASC
             LIMIT 1
         )
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(member_id.0)
    .bind(&expires)
    .bind(lease.0)
    .bind(now.to_rfc3339())
    .bind(channel_id.0)
    .bind(now.to_rfc3339())
    .bind(member_id.0)
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
/// 192). `NotFound` if the thread is gone or the caller isn't the holder — so a
/// member can't renew a lease it doesn't own.
pub async fn renew_claim(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
    lease_secs: i64,
) -> Result<Thread, StoreError> {
    let now = Utc::now();
    let expires = (now + chrono::Duration::seconds(lease_secs)).to_rfc3339();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignment_expires_at = ?, updated_at = ?
         WHERE id = ? AND assignee_id = ? AND claim_lease_id = ? AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(&expires)
    .bind(now.to_rfc3339())
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Stamp the working clock (Cluster 351): the current holder acknowledges the
/// claim and begins work. Fenced by `(assignee_id, claim_lease_id)`; `COALESCE`
/// keeps the first start time (idempotent re-ack within a claim epoch; a reclaim
/// reset it to NULL). `NotFound` if the caller isn't the holder or the token is
/// stale.
pub async fn acknowledge_claim(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<Thread, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "UPDATE maidan_threads SET work_started_at = COALESCE(work_started_at, ?), updated_at = ?
         WHERE id = ? AND assignee_id = ? AND claim_lease_id = ? AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(&now)
    .bind(&now)
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Release a claim (graceful handoff, Cluster 351): the current holder returns
/// the thread to the queue immediately. Fenced by `(assignee_id, claim_lease_id)`;
/// clears the assignee, lease, and working clock. See the Postgres twin.
pub async fn release_claim(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, claim_lease_id = NULL, work_started_at = NULL, updated_at = ?
         WHERE id = ? AND assignee_id = ? AND claim_lease_id = ? AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Release a claim and append its `ThreadAssignmentChanged` event in one tx
/// (Cluster 351). The previous assignee is the caller (the fence guarantees it).
pub async fn release_claim_with_event(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<(Thread, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, claim_lease_id = NULL, work_started_at = NULL, updated_at = ?
         WHERE id = ? AND assignee_id = ? AND claim_lease_id = ? AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let thread = row_to_thread(&row)?;
    let stored =
        append_assignment_event(&mut tx, &thread, member_id, Some(member_id), None).await?;
    tx.commit().await?;
    Ok((thread, stored))
}

pub async fn list_for_workspace(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at
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
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at
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

/// One keyset page of a channel's **live** threads, ordered `(created_at, id)`
/// ascending (Cluster 343). `after` is an exclusive cursor (the prior page's last
/// thread id); `None` starts from the beginning. The channel-scoped twin of
/// [`page_for_workspace`] — bounds the previously-unbounded channel thread list.
pub async fn page_for_channel(
    pool: &SqlitePool,
    channel_id: ChannelId,
    after: Option<ThreadId>,
    limit: i64,
) -> Result<Vec<Thread>, StoreError> {
    let cursor = after.map(|t| t.0);
    let rows = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at,
                tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at
         FROM maidan_threads t
         WHERE t.channel_id = ?
           AND t.tombstoned_at IS NULL
           AND (? IS NULL OR (t.created_at, t.id) > (
                 SELECT ct.created_at, ct.id FROM maidan_threads ct WHERE ct.id = ?
               ))
         ORDER BY t.created_at ASC, t.id ASC
         LIMIT ?",
    )
    .bind(channel_id.0)
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
