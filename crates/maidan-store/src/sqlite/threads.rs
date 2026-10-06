use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, ChannelOccupancy, ChildThreadSummary, ClaimLeaseId, Event, MemberId, NewThread,
    QueueDepth, SpawnAxis, SpawnDenial, StoredEvent, Thread, ThreadClaimResult, ThreadId,
    ThreadState, WorkspaceId, DM_CHANNEL_NAME,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::claim_next::{self, ClaimScope, ClaimSql};
use crate::queue_counts::{self, QueueScope, QueueSql};
use crate::sqlite::budget;
use crate::sqlite::events;
use crate::sqlite::thread_workers;

use crate::error::StoreError;

pub async fn create(pool: &SqlitePool, new: NewThread) -> Result<Thread, StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    enforce_spawn_budget(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::now_v7();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title, description, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(id)
    .bind(new.channel_id.0)
    .bind(new.parent_thread_id.map(|p| p.0))
    .bind(new.title.as_deref())
    .bind(new.description.as_deref())
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .fetch_one(pool)
    .await?;
    row_to_thread(&row)
}

/// Insert a thread and append its `ThreadCreated` event in one transaction. The
/// workspace is resolved in the same tx.
pub async fn create_with_event(
    pool: &SqlitePool,
    new: NewThread,
) -> Result<(Thread, StoredEvent), StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    enforce_spawn_budget(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::now_v7();
    let now = Utc::now();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title, description, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(id)
    .bind(new.channel_id.0)
    .bind(new.parent_thread_id.map(|p| p.0))
    .bind(new.title.as_deref())
    .bind(new.description.as_deref())
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
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.description, t.state, t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id,
                (t.state IN ('closed', 'archived') AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_reviews r
                   WHERE r.thread_id = t.id AND r.decision = 'approve' AND r.dismissed_at IS NULL
                 )) AS closed_without_review
         FROM maidan_threads t WHERE t.id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_board_thread(&row)
}

pub async fn list(pool: &SqlitePool, channel_id: ChannelId) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id
         FROM maidan_threads WHERE channel_id = ? ORDER BY created_at DESC",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// Seconds an acknowledged claim worked up to `ended`. `None` when the claim
/// was never acknowledged: that charges nothing.
fn worked_secs(started: Option<DateTime<Utc>>, ended: DateTime<Utc>) -> Option<i64> {
    started.map(|start| (ended - start).num_seconds().max(0))
}

/// Charge the claim this thread still holds. SQLite stamps `work_started_at`
/// from the host clock, so the end is that same clock. An unacknowledged
/// claim is charged nothing. `NotFound` when the thread is gone.
pub(crate) async fn charge_open_claim_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    let row = sqlx::query("SELECT work_started_at FROM maidan_threads WHERE id = ?")
        .bind(thread_id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    let worked = worked_secs(row.get("work_started_at"), Utc::now());
    budget::charge_wall_in_tx(tx, thread_id, worked).await?;
    Ok(())
}

/// Release every live claim `member_id` holds, charging each acknowledged
/// claim first. See the Postgres twin.
pub(crate) async fn release_member_claims_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    member_id: MemberId,
) -> Result<u64, StoreError> {
    let ended = Utc::now();
    let rows = sqlx::query(
        "SELECT id, work_started_at FROM maidan_threads
         WHERE assignee_id = ? AND tombstoned_at IS NULL
           AND state NOT IN ('closed', 'archived')",
    )
    .bind(member_id.0)
    .fetch_all(&mut **tx)
    .await?;
    for row in &rows {
        let worked = worked_secs(row.get("work_started_at"), ended);
        budget::charge_wall_in_tx(tx, ThreadId(row.get("id")), worked).await?;
    }
    let now = ended.to_rfc3339();
    let released = sqlx::query(
        "UPDATE maidan_threads
         SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL,
             claimed_at = NULL, work_started_at = NULL, updated_at = ?
         WHERE assignee_id = ? AND tombstoned_at IS NULL
           AND state NOT IN ('closed', 'archived')",
    )
    .bind(&now)
    .bind(member_id.0)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(released)
}

const CLEAR_CLAIM: &str = "UPDATE maidan_threads SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = ?
         WHERE id = ?
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id";

async fn clear_assignee_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<Thread, StoreError> {
    charge_open_claim_in_tx(tx, thread_id).await?;
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(CLEAR_CLAIM)
        .bind(&now)
        .bind(thread_id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

async fn release_fenced_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "SELECT work_started_at FROM maidan_threads
         WHERE id = ? AND assignee_id = ? AND claim_lease_id = ? AND tombstoned_at IS NULL",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let worked = worked_secs(row.get("work_started_at"), Utc::now());
    budget::charge_wall_in_tx(tx, thread_id, worked).await?;
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = ?
         WHERE id = ? AND assignee_id = ? AND claim_lease_id = ? AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(&now)
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Set the assignee unconditionally (assign / handoff). `NotFound` if absent or
/// tombstoned.
pub async fn assign(
    pool: &SqlitePool,
    thread_id: ThreadId,
    assignee_id: MemberId,
) -> Result<Thread, StoreError> {
    let lease = ClaimLeaseId::new();
    // In a transaction only so the worker record commits with the assignment.
    // This variant emits no event, so it does not pass through
    // `append_assignment_event` where the other paths record.
    let mut tx = pool.begin().await?;
    // A reassignment ends the claim it replaces. Charge it before the write
    // clears the working clock; a free thread charges nothing.
    charge_open_claim_in_tx(&mut tx, thread_id).await?;
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, assignment_expires_at = NULL, claim_lease_id = ?, work_started_at = NULL, updated_at = ?, claimed_at = ?
         WHERE id = ? AND tombstoned_at IS NULL AND EXISTS (SELECT 1 FROM maidan_members m JOIN maidan_channels c ON c.workspace_id = m.workspace_id WHERE m.id = ? AND c.id = maidan_threads.channel_id)
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(assignee_id.0)
    .bind(lease.0)
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .bind(assignee_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    thread_workers::record_in_tx(&mut tx, thread_id, assignee_id).await?;
    tx.commit().await?;
    row_to_thread(&row)
}

/// A parent thread's child threads, collapsed with a live message count each.
/// Oldest first; tombstoned children excluded.
pub async fn child_summaries(
    pool: &SqlitePool,
    parent_id: ThreadId,
) -> Result<Vec<ChildThreadSummary>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.description, t.state, t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id,
                (SELECT COUNT(*) FROM maidan_messages m WHERE m.thread_id = t.id AND m.tombstoned_at IS NULL) AS message_count
         FROM maidan_threads t
         WHERE t.parent_thread_id = ? AND t.tombstoned_at IS NULL
         ORDER BY t.created_at ASC, t.id ASC",
    )
    .bind(parent_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(ChildThreadSummary {
                thread: row_to_thread(row)?,
                message_count: row.get::<i64, _>("message_count"),
            })
        })
        .collect()
}

/// A channel's threads ordered by last activity: most-recently bumped first,
/// tombstoned excluded, capped at `limit`. The bump-to-top read.
pub async fn list_recently_active(
    pool: &SqlitePool,
    channel_id: ChannelId,
    limit: i64,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        // `strftime('%Y-%m-%d %H:%M:%f', updated_at)` normalizes the two stored
        // forms — `datetime('now')` (space, second precision, from the create
        // default) and `to_rfc3339()` (T + zone + sub-second, from the bump) — to a
        // common millisecond UTC value, so the recency order is by real time (not
        // the lexical accident that 'T' > ' ') AND a bump outranks a same-second
        // create (which datetime()'s second truncation would tie).
        "SELECT id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id
         FROM maidan_threads
         WHERE channel_id = ? AND tombstoned_at IS NULL
         ORDER BY strftime('%Y-%m-%d %H:%M:%f', updated_at) DESC, id DESC
         LIMIT ?",
    )
    .bind(channel_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// Set (or clear) the durable owner. `NotFound` if absent or tombstoned.
/// Touches only `owner_id` — orthogonal to the claim axis.
pub async fn set_owner(
    pool: &SqlitePool,
    thread_id: ThreadId,
    owner_id: Option<MemberId>,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET owner_id = ?, updated_at = ?
         WHERE id = ? AND tombstoned_at IS NULL
           AND (? IS NULL OR EXISTS (SELECT 1 FROM maidan_members m JOIN maidan_channels c ON c.workspace_id = m.workspace_id WHERE m.id = ? AND c.id = maidan_threads.channel_id))
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(owner_id.map(|o| o.0))
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .bind(owner_id.map(|o| o.0))
    .bind(owner_id.map(|o| o.0))
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Rename a thread. `NotFound` if absent or tombstoned. Touches only `title` —
/// a rename is metadata, not activity, so it does not bump `updated_at` (the
/// activity-sort key).
pub async fn set_title(
    pool: &SqlitePool,
    thread_id: ThreadId,
    title: Option<String>,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET title = ?
         WHERE id = ? AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(title)
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Assign a thread and append its `ThreadAssignmentChanged` event in one
/// transaction. The previous assignee is captured in the same tx (a consistent
/// read, not a separate `get_thread` + race window).
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
    charge_open_claim_in_tx(&mut tx, thread_id).await?;
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, assignment_expires_at = NULL, claim_lease_id = ?, work_started_at = NULL, updated_at = ?, claimed_at = ?
         WHERE id = ? AND tombstoned_at IS NULL AND EXISTS (SELECT 1 FROM maidan_members m JOIN maidan_channels c ON c.workspace_id = m.workspace_id WHERE m.id = ? AND c.id = maidan_threads.channel_id)
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(assignee_id.0)
    .bind(lease.0)
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .bind(assignee_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let thread = row_to_thread(&row)?;
    let stored = append_assignment_event(&mut tx, &thread, actor_id, previous, note).await?;
    tx.commit().await?;
    Ok((thread, stored))
}

/// Clear the assignee. `NotFound` if absent.
pub async fn unassign(pool: &SqlitePool, thread_id: ThreadId) -> Result<Thread, StoreError> {
    let mut tx = pool.begin().await?;
    let thread = clear_assignee_in_tx(&mut tx, thread_id).await?;
    tx.commit().await?;
    Ok(thread)
}

/// Clear the assignee and append its `ThreadAssignmentChanged` event in one
/// transaction. No handoff note (unassign carries none).
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
    let thread = clear_assignee_in_tx(&mut tx, thread_id).await?;
    let stored = append_assignment_event(&mut tx, &thread, actor_id, previous, None).await?;
    tx.commit().await?;
    Ok((thread, stored))
}

/// Build + append a `ThreadAssignmentChanged` event on a caller-supplied tx.
/// Shared by the assignment `*_with_event` mutations.
async fn append_assignment_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread: &Thread,
    actor_id: MemberId,
    previous_assignee_id: Option<MemberId>,
    note: Option<String>,
) -> Result<StoredEvent, StoreError> {
    // Every path that hands a thread to someone comes through here, so this is
    // where the durable worker record is written — beside the event rather than
    // at each of the three call sites, because a separation-of-duties control
    // that one call site can forget is not a control. On the caller's tx: a
    // ledger row lost while the assignment commits fails *open*, letting the
    // worker approve their own work.
    if let Some(assignee) = thread.assignee_id {
        thread_workers::record_in_tx(tx, thread.id, assignee).await?;
    }
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

/// Build + append a `ClaimExpired` event on a caller-supplied tx: the previous
/// holder `expired_member`'s lease lapsed and the thread was reclaimed. See the
/// Postgres twin.
async fn append_claim_expired_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread: &Thread,
    expired_member: MemberId,
) -> Result<StoredEvent, StoreError> {
    let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, thread.id).await?;
    let event = Event::ClaimExpired {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id: thread.id,
        member_id: expired_member,
        thread: thread.clone(),
    };
    events::append_in_tx(tx, &event).await
}

/// End a claim whose lease lapsed, after the write that took the thread off
/// `holder`: charge its worked time to the wall budget, then report it as
/// failed (over budget) or expired. See the Postgres twin.
async fn end_lapsed_claim_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread: &Thread,
    holder: MemberId,
    deadline: DateTime<Utc>,
    work_started_at: Option<DateTime<Utc>>,
) -> Result<StoredEvent, StoreError> {
    let worked = work_started_at.map(|started| (deadline - started).num_seconds().max(0));
    let charged = budget::charge_wall_in_tx(tx, thread.id, worked).await?;
    match charged.and_then(|b| b.exceeded(None).map(|reason| (b, reason))) {
        Some((charged, reason)) => {
            let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, thread.id).await?;
            budget::fail_claim_in_tx(
                tx,
                workspace_id,
                channel_id,
                thread.clone(),
                holder,
                reason,
                &charged,
            )
            .await
        }
        None => append_claim_expired_event(tx, thread, holder).await,
    }
}

/// The holder, deadline and working clock of the claim a candidate row still
/// carries (`assignee_id`, `assignment_expires_at`, `work_started_at`), read
/// before the update that takes it over. `None` when it was unassigned.
fn lapsed_claim(
    row: &sqlx::sqlite::SqliteRow,
) -> Option<(MemberId, DateTime<Utc>, Option<DateTime<Utc>>)> {
    let holder = row.get::<Option<Uuid>, _>("assignee_id").map(MemberId)?;
    let deadline = row.get::<Option<DateTime<Utc>>, _>("assignment_expires_at")?;
    Some((holder, deadline, row.get("work_started_at")))
}

/// The thread `claim_next` would give `member_id` in `scope` at `now`, with
/// the claim it still carries (see [`lapsed_claim`]). Shared by both
/// `claim_next` variants and both scopes so they take the same thread.
async fn claim_next_candidate(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    scope: ClaimScope,
    member_id: MemberId,
    now: DateTime<Utc>,
) -> Result<Option<sqlx::sqlite::SqliteRow>, StoreError> {
    let sql = claim_next::candidate_select(
        scope,
        "cand.id, cand.assignee_id, cand.assignment_expires_at, cand.work_started_at",
        &ClaimSql {
            member: "?1",
            scope: "?2",
            now: "?3",
            dm_channel: "?4",
            hours_waiting:
                "CAST((strftime('%s','now') - strftime('%s', cand.created_at)) / 3600 AS INTEGER)",
            lapsed_worked_secs: "CASE WHEN cand.assignee_id IS NOT NULL AND cand.work_started_at IS NOT NULL AND cand.assignment_expires_at IS NOT NULL THEN MAX(0, CAST(strftime('%s', cand.assignment_expires_at) AS INTEGER) - CAST(strftime('%s', cand.work_started_at) AS INTEGER)) ELSE 0 END",
        },
    );
    Ok(sqlx::query(&sql)
        .bind(member_id.0)
        .bind(scope.id())
        .bind(now.to_rfc3339())
        .bind(DM_CHANNEL_NAME)
        .fetch_optional(&mut **tx)
        .await?)
}

/// Give `candidate` to `member_id` under a new fencing token. The caller
/// chose it with [`claim_next_candidate`] in the same transaction, and SQLite
/// serializes writers, so it is still claimable.
async fn take_candidate(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    candidate: &sqlx::sqlite::SqliteRow,
    member_id: MemberId,
    now: DateTime<Utc>,
    lease_secs: Option<i64>,
) -> Result<Thread, StoreError> {
    let expires = lease_secs.map(|s| (now + chrono::Duration::seconds(s)).to_rfc3339());
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, assignment_expires_at = ?, claim_lease_id = ?, work_started_at = NULL, updated_at = ?, claimed_at = ?
         WHERE id = ?
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(member_id.0)
    .bind(&expires)
    .bind(ClaimLeaseId::new().0)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(candidate.get::<Uuid, _>("id"))
    .fetch_one(&mut **tx)
    .await?;
    row_to_thread(&row)
}

/// Atomic compare-and-set claim: `assignee_id IS NULL` guards the UPDATE so
/// only one concurrent claimer wins. `None` → already assigned (or absent);
/// disambiguate with a follow-up read.
pub async fn claim(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<ThreadClaimResult, StoreError> {
    let lease = ClaimLeaseId::new();
    // See `assign`: the transaction exists so the worker record commits with
    // the claim. Only a *winning* claim records — a losing compare-and-set
    // never held the thread.
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, assignment_expires_at = NULL, claim_lease_id = ?, work_started_at = NULL, updated_at = ?, claimed_at = ?
         WHERE id = ? AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(member_id.0)
    .bind(lease.0)
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?;
    match row {
        Some(row) => {
            thread_workers::record_in_tx(&mut tx, thread_id, member_id).await?;
            tx.commit().await?;
            Ok(ThreadClaimResult {
                thread: row_to_thread(&row)?,
                claimed: true,
            })
        }
        None => {
            tx.commit().await?;
            Ok(ThreadClaimResult {
                thread: get(pool, thread_id).await?,
                claimed: false,
            })
        }
    }
}

/// Atomic claim + its `ThreadAssignmentChanged` event in one tx. Conditional:
/// the event is appended **only** when the CAS actually claimed (`(result,
/// Some)`); an already-assigned thread yields `(result{claimed:false}, None)` —
/// no event. `previous_assignee_id` is `None` (plain claim guards on
/// unassigned).
pub async fn claim_with_event(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<(ThreadClaimResult, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = ?, assignment_expires_at = NULL, claim_lease_id = ?, work_started_at = NULL, updated_at = ?, claimed_at = ?
         WHERE id = ? AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(member_id.0)
    .bind(lease.0)
    .bind(Utc::now().to_rfc3339())
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

/// Threads in `workspace_id` currently assigned to `member_id` — an agent's
/// work queue. Live threads only, oldest first. Uses `idx_threads_assignee`.
pub async fn list_assigned(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.description, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
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

/// Threads under review in `workspace_id` that name `member_id` as a reviewer
/// and do not have that member's approval yet: the reviews waiting on them,
/// oldest first.
pub async fn list_review_requests(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.description, t.state,
                t.created_at, COALESCE(
                    (SELECT MAX(tt.occurred_at) FROM maidan_thread_transitions tt
                     WHERE tt.thread_id = t.id AND tt.to_state = 'in_review'),
                    t.updated_at) AS updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         JOIN maidan_thread_reviewers rv ON rv.thread_id = t.id AND rv.member_id = ?
         -- review_since: when the thread last entered review, not updated_at,
         -- which a claim renewal or a rename bumps. row_to_thread reads it as
         -- updated_at, so the inbox ages the request from when review began.
         WHERE c.workspace_id = ? AND t.state = 'in_review' AND t.tombstoned_at IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM maidan_thread_reviews r
             WHERE r.thread_id = t.id AND r.reviewer_id = ? AND r.decision = 'approve'
               AND r.dismissed_at IS NULL
           )
         -- Oldest review first, by when the thread last entered review: a claim
         -- renewal or other touch bumps updated_at but not its place in line.
         ORDER BY COALESCE(
                    (SELECT MAX(tt.occurred_at) FROM maidan_thread_transitions tt
                     WHERE tt.thread_id = t.id AND tt.to_state = 'in_review'),
                    t.updated_at) ASC, t.id ASC",
    )
    .bind(member_id.0)
    .bind(workspace_id.0)
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// Threads under review in `workspace_id` with no named reviewer: the ones
/// `member_id` owns, and, when `include_ownerless`, the ones nobody owns.
/// Oldest first, by when the thread last entered review.
pub async fn list_unassigned_reviews(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    include_ownerless: bool,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.description, t.state,
                t.created_at, COALESCE(
                    (SELECT MAX(tt.occurred_at) FROM maidan_thread_transitions tt
                     WHERE tt.thread_id = t.id AND tt.to_state = 'in_review'),
                    t.updated_at) AS updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ? AND t.state = 'in_review' AND t.tombstoned_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM maidan_thread_reviewers rv WHERE rv.thread_id = t.id)
           AND (t.owner_id = ? OR (? AND t.owner_id IS NULL))
           -- An approval this member already gave answers it for them.
           AND NOT EXISTS (
             SELECT 1 FROM maidan_thread_reviews r
             WHERE r.thread_id = t.id AND r.reviewer_id = ? AND r.decision = 'approve'
               AND r.dismissed_at IS NULL
           )
         ORDER BY COALESCE(
                    (SELECT MAX(tt.occurred_at) FROM maidan_thread_transitions tt
                     WHERE tt.thread_id = t.id AND tt.to_state = 'in_review'),
                    t.updated_at) ASC, t.id ASC",
    )
    .bind(workspace_id.0)
    .bind(member_id.0)
    .bind(include_ownerless)
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// Atomically claim the oldest unassigned live thread in `channel_id` for
/// `member_id` — the "pull the next task" primitive. `None` when the channel
/// has no unassigned work. SQLite serializes writers, so the select-then-update
/// can't double-assign.
pub async fn claim_next(
    pool: &SqlitePool,
    channel_id: ChannelId,
    member_id: MemberId,
    lease_secs: Option<i64>,
) -> Result<Option<Thread>, StoreError> {
    let now = Utc::now();
    // Claimable = unassigned OR the current lease has expired (dead-agent
    // recovery). SQLite serializes writers so the select-then-update is
    // race-free. The transaction exists so the worker record commits with the
    // claim.
    let mut tx = pool.begin().await?;
    let Some(candidate) =
        claim_next_candidate(&mut tx, ClaimScope::Channel(channel_id), member_id, now).await?
    else {
        tx.commit().await?;
        return Ok(None);
    };
    let thread = take_candidate(&mut tx, &candidate, member_id, now, lease_secs).await?;
    thread_workers::record_in_tx(&mut tx, thread.id, member_id).await?;
    // A takeover of a lapsed lease ends that claim here, charged as the reaper
    // would charge it; this variant returns no events, but the log still
    // records the end.
    if let Some((holder, deadline, started)) = lapsed_claim(&candidate) {
        end_lapsed_claim_in_tx(&mut tx, &thread, holder, deadline, started).await?;
    }
    tx.commit().await?;
    Ok(Some(thread))
}

/// The placeholders the queue counts bind on SQLite: `?1` the scope, `?2` the
/// reader, `?3` the DM channel name, `?4` now.
const QUEUE_SQL: QueueSql<'static> = QueueSql {
    scope: "?1",
    reader: "?2",
    dm_channel: "?3",
    now: "?4",
};

/// Task-queue depth for a channel or a workspace — see
/// [`queue_counts`](crate::queue_counts). Its `available` test and the
/// dependency and explicit-block clauses mirror the `claim_next` claimability
/// predicate. `blocked` folds an explicit [`BlockedReason`] in alongside
/// unfinished DAG deps — they stay distinct *causes*.
///
/// [`BlockedReason`]: maidan_types::BlockedReason
pub async fn queue_depth(
    pool: &SqlitePool,
    scope: QueueScope,
    readable_by: Option<MemberId>,
) -> Result<QueueDepth, StoreError> {
    let row = sqlx::query(&queue_counts::queue_depth_select(scope, &QUEUE_SQL))
        .bind(scope.id())
        .bind(readable_by.map(|m| m.0))
        .bind(DM_CHANNEL_NAME)
        .bind(Utc::now().to_rfc3339())
        .fetch_one(pool)
        .await?;
    Ok(QueueDepth {
        open: row.get::<i64, _>("open_count"),
        ready: row.get::<i64, _>("ready_count"),
        assigned: row.get::<i64, _>("assigned_count"),
        blocked: row.get::<i64, _>("blocked_count"),
        unclaimable: row.get::<i64, _>("unclaimable_count"),
    })
}

/// Occupancy of a channel or a workspace — the two-clocks refinement of
/// [`queue_depth`], splitting the held threads by the working clock. See the
/// Postgres twin. The four sub-counts partition `open`.
pub async fn occupancy(
    pool: &SqlitePool,
    scope: QueueScope,
    readable_by: Option<MemberId>,
) -> Result<ChannelOccupancy, StoreError> {
    let row = sqlx::query(&queue_counts::occupancy_select(scope, &QUEUE_SQL))
        .bind(scope.id())
        .bind(readable_by.map(|m| m.0))
        .bind(DM_CHANNEL_NAME)
        .bind(Utc::now().to_rfc3339())
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

/// Atomic claim-next in `scope` + its `ThreadAssignmentChanged` event in one
/// tx. Conditional: the event is appended **only** when a thread was claimed
/// (`(Some(thread), events)`); nothing to claim yields `(None, [])`.
/// `previous_assignee_id` is `None` (behaviour-preserving — matches the old
/// route; a takeover of a lapsed lease reports the prior holder's end first).
pub(crate) async fn claim_next_with_event(
    pool: &SqlitePool,
    scope: ClaimScope,
    member_id: MemberId,
    lease_secs: Option<i64>,
) -> Result<(Option<Thread>, Vec<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();
    // SQLite serializes writers, so a select-then-update in one tx is race-free.
    // The candidate row keeps its pre-update claim, so a reclaim of an expired
    // lease can end the dead holder's claim (RETURNING would only give the
    // post-update row).
    let Some(candidate) = claim_next_candidate(&mut tx, scope, member_id, now).await? else {
        tx.commit().await?;
        return Ok((None, Vec::new()));
    };
    let thread = take_candidate(&mut tx, &candidate, member_id, now, lease_secs).await?;
    let mut events = Vec::new();
    if let Some((holder, deadline, started)) = lapsed_claim(&candidate) {
        events.push(end_lapsed_claim_in_tx(&mut tx, &thread, holder, deadline, started).await?);
    }
    events.push(append_assignment_event(&mut tx, &thread, member_id, None, None).await?);
    tx.commit().await?;
    Ok((Some(thread), events))
}

/// Reap up to `limit` lapsed leases on open, live threads, charging each
/// claim's worked time to its wall budget and appending a `ClaimExpired` (or,
/// over budget, a `ClaimFailed`) for each dead holder, all in one transaction.
/// SQLite serializes writers, so the select-then-update cannot race
/// `claim_next` or another reaper.
pub async fn reap_expired_claims(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut tx = pool.begin().await?;
    let lapsed = sqlx::query(
        "SELECT id, assignee_id, assignment_expires_at, work_started_at FROM maidan_threads
         WHERE assignee_id IS NOT NULL
           AND assignment_expires_at IS NOT NULL AND assignment_expires_at < ?
           AND tombstoned_at IS NULL AND state = 'open'
         ORDER BY assignment_expires_at ASC, id ASC
         LIMIT ?",
    )
    .bind(now.to_rfc3339())
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let mut events = Vec::with_capacity(lapsed.len());
    for candidate in &lapsed {
        let id: Uuid = candidate.get("id");
        let (holder, deadline, started) = lapsed_claim(candidate).ok_or_else(|| {
            StoreError::InvalidInput("a reaped claim has no holder or deadline".into())
        })?;
        // Guarded on the same holder and a still-lapsed lease, so a claim
        // taken or renewed since the read above is left alone.
        let Some(row) = sqlx::query(
            "UPDATE maidan_threads SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = ?
             WHERE id = ? AND assignee_id = ? AND assignment_expires_at < ?
               AND tombstoned_at IS NULL AND state = 'open'
             RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .bind(holder.0)
        .bind(now.to_rfc3339())
        .fetch_optional(&mut *tx)
        .await?
        else {
            continue;
        };
        let thread = row_to_thread(&row)?;
        events.push(end_lapsed_claim_in_tx(&mut tx, &thread, holder, deadline, started).await?);
    }
    tx.commit().await?;
    Ok(events)
}

/// Report leased claims taken before `claimed_before` that their holder never
/// acknowledged, once per claim. See [`Store::report_unacknowledged_claims`].
/// Each update is guarded on the same token and a still-unacknowledged claim,
/// so a claim acknowledged, released or reported since the read is skipped.
pub async fn report_unacknowledged_claims(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    claimed_before: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    const STALE: &str = "claim_lease_id IS NOT NULL AND work_started_at IS NULL
           AND assignee_id IS NOT NULL
           AND claimed_at IS NOT NULL AND claimed_at < ?
           AND assignment_expires_at IS NOT NULL AND assignment_expires_at >= ?
           AND unacknowledged_lease_id IS NOT claim_lease_id
           AND tombstoned_at IS NULL AND state = 'open'";
    let mut tx = pool.begin().await?;
    let candidates = sqlx::query(&format!(
        "SELECT id, claim_lease_id FROM maidan_threads WHERE {STALE}
         ORDER BY claimed_at ASC, id ASC
         LIMIT ?"
    ))
    .bind(claimed_before.to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let mut events = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        let id: Uuid = candidate.get("id");
        let lease: Uuid = candidate.get("claim_lease_id");
        let Some(row) = sqlx::query(&format!(
            "UPDATE maidan_threads SET unacknowledged_lease_id = claim_lease_id
             WHERE id = ? AND claim_lease_id = ? AND {STALE}
             RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, claimed_at"
        ))
        .bind(id)
        .bind(lease)
        .bind(claimed_before.to_rfc3339())
        .bind(now.to_rfc3339())
        .fetch_optional(&mut *tx)
        .await?
        else {
            continue;
        };
        let thread = row_to_thread(&row)?;
        let claimed_at: DateTime<Utc> = row.get("claimed_at");
        events.push(append_claim_unacknowledged_event(&mut tx, &thread, claimed_at).await?);
    }
    tx.commit().await?;
    Ok(events)
}

async fn append_claim_unacknowledged_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread: &Thread,
    claimed_at: DateTime<Utc>,
) -> Result<StoredEvent, StoreError> {
    let member_id = thread
        .assignee_id
        .ok_or_else(|| StoreError::InvalidInput("an unacknowledged claim has no holder".into()))?;
    let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, thread.id).await?;
    let event = Event::ClaimUnacknowledged {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id: thread.id,
        member_id,
        claimed_at,
        thread: thread.clone(),
    };
    events::append_in_tx(tx, &event).await
}

/// Extend a claim's lease (heartbeat), only for the current assignee.
/// `NotFound` if the thread is gone or the caller isn't the holder — so a
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
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
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

/// Stamp the working clock: the current holder acknowledges the claim and
/// begins work. Fenced by `(assignee_id, claim_lease_id)`; `COALESCE` keeps the
/// first start time (idempotent re-ack within a claim epoch; a reclaim reset it
/// to NULL). `NotFound` if the caller isn't the holder or the token is stale.
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
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
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

/// Release a claim (graceful handoff): the current holder returns the thread to
/// the queue immediately. Fenced by `(assignee_id, claim_lease_id)`; clears the
/// assignee, lease, and working clock. See the Postgres twin.
pub async fn release_claim(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<Thread, StoreError> {
    let mut tx = pool.begin().await?;
    let thread = release_fenced_in_tx(&mut tx, thread_id, member_id, lease_id).await?;
    tx.commit().await?;
    Ok(thread)
}

/// Release a claim and append its `ThreadAssignmentChanged` event in one tx.
/// The previous assignee is the caller (the fence guarantees it).
pub async fn release_claim_with_event(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<(Thread, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let thread = release_fenced_in_tx(&mut tx, thread_id, member_id, lease_id).await?;
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
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.description, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
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
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.description, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
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
/// ascending. `after` is an exclusive cursor (the prior page's last thread id);
/// `None` starts from the beginning. The channel-scoped twin of
/// [`page_for_workspace`] — bounds the previously-unbounded channel thread
/// list.
pub async fn page_for_channel(
    pool: &SqlitePool,
    channel_id: ChannelId,
    after: Option<ThreadId>,
    limit: i64,
) -> Result<Vec<Thread>, StoreError> {
    let cursor = after.map(|t| t.0);
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.description, t.state, t.created_at, t.updated_at,
                t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id,
                (t.state IN ('closed', 'archived') AND NOT EXISTS (
                   SELECT 1 FROM maidan_thread_reviews r
                   WHERE r.thread_id = t.id AND r.decision = 'approve' AND r.dismissed_at IS NULL
                 )) AS closed_without_review
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
    rows.iter().map(row_to_board_thread).collect()
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

/// Enforce the workspace's spawn budget — refuse a child once the parent holds
/// `max_children` or its nesting would exceed `max_depth`. Root threads /
/// no-budget workspaces are unrestricted. A refusal is a typed `SpawnRejected`:
/// a 409/InvalidParams for the caller, and the payload the route publishes as
/// `ThreadSpawnDenied`.
async fn enforce_spawn_budget(
    pool: &SqlitePool,
    channel_id: ChannelId,
    parent_thread_id: Option<ThreadId>,
) -> Result<(), StoreError> {
    let Some(parent_id) = parent_thread_id else {
        return Ok(());
    };
    let Some(ws_row) = sqlx::query("SELECT workspace_id FROM maidan_channels WHERE id = ?")
        .bind(channel_id.0)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(());
    };
    let workspace_id = maidan_types::WorkspaceId(ws_row.get::<Uuid, _>("workspace_id"));
    let Some(budget) = super::spawn::get_budget(pool, workspace_id).await? else {
        return Ok(());
    };
    let refuse = |axis, limit, observed| {
        Err(StoreError::spawn_rejected(SpawnDenial {
            workspace_id,
            channel_id,
            thread_id: parent_id,
            axis,
            limit,
            observed,
        }))
    };
    if let Some(max_children) = budget.max_children {
        let children = super::spawn::count_active_children(pool, parent_id).await?;
        if children >= max_children {
            return refuse(SpawnAxis::Children, max_children, children);
        }
    }
    if let Some(max_depth) = budget.max_depth {
        let depth = super::spawn::thread_depth(pool, parent_id).await?;
        if depth >= max_depth {
            return refuse(SpawnAxis::Depth, max_depth, depth);
        }
    }
    Ok(())
}

/// A thread read for a board: the row plus its `closed_without_review` column,
/// which only the board's reads select.
fn row_to_board_thread(row: &sqlx::sqlite::SqliteRow) -> Result<Thread, StoreError> {
    let mut thread = row_to_thread(row)?;
    thread.closed_without_review = row.get("closed_without_review");
    Ok(thread)
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
    let owner: Option<Uuid> = row.get("owner_id");
    Ok(Thread {
        id: ThreadId(row.get::<Uuid, _>("id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        parent_thread_id: parent.map(ThreadId),
        title: row.get("title"),
        // `try_get`: not every SELECT that feeds this mapper lists the
        // column (the 0141 migration added it after most were written);
        // those rows read as undescribed rather than failing.
        description: row.try_get::<Option<String>, _>("description").unwrap_or(None),
        state,
        assignee_id: assignee.map(MemberId),
        assignment_expires_at: row.get::<Option<DateTime<Utc>>, _>("assignment_expires_at"),
        claim_lease_id: row
            .get::<Option<Uuid>, _>("claim_lease_id")
            .map(ClaimLeaseId),
        work_started_at: row.get::<Option<DateTime<Utc>>, _>("work_started_at"),
        owner_id: owner.map(MemberId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
        status: None,
        block: None,
        closed_without_review: false,
    })
}
