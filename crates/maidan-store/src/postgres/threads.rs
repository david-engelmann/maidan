use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, ChannelOccupancy, ChildThreadSummary, ClaimLeaseId, Event, MemberId, NewThread,
    QueueDepth, SpawnAxis, SpawnDenial, StoredEvent, Thread, ThreadClaimResult, ThreadId,
    ThreadState, WorkspaceId, DM_CHANNEL_NAME,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::claim_next::{self, ClaimScope, ClaimSql};
use crate::error::StoreError;
use crate::postgres::budget;
use crate::postgres::events;
use crate::postgres::thread_workers;

pub async fn create(pool: &PgPool, new: NewThread) -> Result<Thread, StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    enforce_spawn_budget(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::now_v7();
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title)
         VALUES ($1, $2, $3, $4)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(id)
    .bind(new.channel_id.0)
    .bind(new.parent_thread_id.map(|p| p.0))
    .bind(new.title.as_deref())
    .fetch_one(pool)
    .await?;
    row_to_thread(&row)
}

/// Insert a thread and append its `ThreadCreated` event in one transaction —
/// see the SQLite twin.
pub async fn create_with_event(
    pool: &PgPool,
    new: NewThread,
) -> Result<(Thread, StoredEvent), StoreError> {
    validate_parent(pool, new.channel_id, new.parent_thread_id).await?;
    enforce_spawn_budget(pool, new.channel_id, new.parent_thread_id).await?;
    let id = Uuid::now_v7();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title)
         VALUES ($1, $2, $3, $4)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
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
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id
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
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id
         FROM maidan_threads WHERE channel_id = $1 ORDER BY created_at DESC",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// Set the assignee unconditionally (assign / handoff). `NotFound` if absent or
/// tombstoned — claiming dead work is a bug.
pub async fn assign(
    pool: &PgPool,
    thread_id: ThreadId,
    assignee_id: MemberId,
) -> Result<Thread, StoreError> {
    let lease = ClaimLeaseId::new();
    // In a transaction only so the worker record commits with the assignment.
    // This variant emits no event, so it does not pass through
    // `append_assignment_event` where the other paths record.
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = $1, assignment_expires_at = NULL, claim_lease_id = $3, claimed_at = NOW(), work_started_at = NULL, updated_at = NOW()
         WHERE id = $2 AND tombstoned_at IS NULL AND EXISTS (SELECT 1 FROM maidan_members m JOIN maidan_channels c ON c.workspace_id = m.workspace_id WHERE m.id = $1 AND c.id = maidan_threads.channel_id)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(assignee_id.0)
    .bind(thread_id.0)
    .bind(lease.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    thread_workers::record_in_tx(&mut tx, thread_id, assignee_id).await?;
    tx.commit().await?;
    row_to_thread(&row)
}

/// Assign a thread and append its `ThreadAssignmentChanged` event in one
/// transaction. The previous assignee is captured in the same tx.
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
        "UPDATE maidan_threads SET assignee_id = $1, assignment_expires_at = NULL, claim_lease_id = $3, claimed_at = NOW(), work_started_at = NULL, updated_at = NOW()
         WHERE id = $2 AND tombstoned_at IS NULL AND EXISTS (SELECT 1 FROM maidan_members m JOIN maidan_channels c ON c.workspace_id = m.workspace_id WHERE m.id = $1 AND c.id = maidan_threads.channel_id)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
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

/// A parent thread's child threads, collapsed with a live message count each.
/// Oldest first; tombstoned children excluded.
pub async fn child_summaries(
    pool: &PgPool,
    parent_id: ThreadId,
) -> Result<Vec<ChildThreadSummary>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state, t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id,
                (SELECT COUNT(*) FROM maidan_messages m WHERE m.thread_id = t.id AND m.tombstoned_at IS NULL) AS message_count
         FROM maidan_threads t
         WHERE t.parent_thread_id = $1 AND t.tombstoned_at IS NULL
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
/// tombstoned excluded, capped at `limit`. The bump-to-top read — a post
/// touches `updated_at`, floating its thread here. Distinct from the
/// keyset-paginated, creation-ordered `page_for_channel` (whose stable sort key
/// this mutable ordering would break).
pub async fn list_recently_active(
    pool: &PgPool,
    channel_id: ChannelId,
    limit: i64,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id
         FROM maidan_threads
         WHERE channel_id = $1 AND tombstoned_at IS NULL
         ORDER BY updated_at DESC, id DESC
         LIMIT $2",
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
    pool: &PgPool,
    thread_id: ThreadId,
    owner_id: Option<MemberId>,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET owner_id = $1, updated_at = NOW()
         WHERE id = $2 AND tombstoned_at IS NULL
           AND ($1::uuid IS NULL OR EXISTS (SELECT 1 FROM maidan_members m JOIN maidan_channels c ON c.workspace_id = m.workspace_id WHERE m.id = $1 AND c.id = maidan_threads.channel_id))
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(owner_id.map(|o| o.0))
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Rename a thread. `NotFound` if absent or tombstoned. Touches only `title` —
/// a rename is metadata, not activity, so it does not bump `updated_at` (the
/// activity-sort key).
pub async fn set_title(
    pool: &PgPool,
    thread_id: ThreadId,
    title: Option<String>,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET title = $1
         WHERE id = $2 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(title)
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Clear the assignee. `NotFound` if absent.
pub async fn unassign(pool: &PgPool, thread_id: ThreadId) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = NOW()
         WHERE id = $1
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Clear the assignee and append its `ThreadAssignmentChanged` event in one
/// transaction. No handoff note.
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
        "UPDATE maidan_threads SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = NOW()
         WHERE id = $1
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
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

/// Build + append a `ThreadAssignmentChanged` event on a caller-supplied tx.
/// Shared by the assignment `*_with_event` mutations.
async fn append_assignment_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
/// holder `expired_member`'s lease lapsed and the thread was reclaimed. Emitted
/// alongside the reclaim's `ThreadAssignmentChanged`.
async fn append_claim_expired_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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

/// End a claim whose lease lapsed, on the caller's transaction, after the write
/// that took the thread off `holder`: charge the time the claim worked to the
/// thread's wall budget, then report it. The claim worked from the holder's
/// acknowledgement to its lease `deadline`, when it stopped being theirs; one
/// never acknowledged had no working clock and is charged nothing, as a usage
/// report never counts one. Over budget after the charge, the claim fails as a
/// report would have failed it (`ClaimFailed` and a DLQ entry); otherwise it
/// expired (`ClaimExpired`). The caller's write clears the holder, so a lapse
/// is charged once, whichever of the reaper and `claim_next` frees it.
async fn end_lapsed_claim_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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

/// The holder, deadline and working clock a claim had before an update took
/// it over, read from the columns a claim or reap query returns beside the
/// thread (`prev_assignee`, `prev_deadline`, `prev_started`). `None` when the
/// thread was unassigned.
fn lapsed_claim(
    row: &sqlx::postgres::PgRow,
) -> Option<(MemberId, DateTime<Utc>, Option<DateTime<Utc>>)> {
    let holder = row.get::<Option<Uuid>, _>("prev_assignee").map(MemberId)?;
    let deadline = row.get::<Option<DateTime<Utc>>, _>("prev_deadline")?;
    Some((holder, deadline, row.get("prev_started")))
}

/// Atomic compare-and-set claim: the `assignee_id IS NULL` predicate + row lock
/// guarantees only one concurrent claimer wins. A `None` result means the row
/// was already assigned (or absent) — disambiguate with a follow-up read.
pub async fn claim(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<ThreadClaimResult, StoreError> {
    let lease = ClaimLeaseId::new();
    // See `assign`: the transaction exists so the worker record commits with
    // the claim. Only a *winning* claim records — a losing compare-and-set
    // never held the thread.
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = $1, assignment_expires_at = NULL, claim_lease_id = $3, claimed_at = NOW(), work_started_at = NULL, updated_at = NOW()
         WHERE id = $2 AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(member_id.0)
    .bind(thread_id.0)
    .bind(lease.0)
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
/// the event is appended **only** when the CAS actually claimed.
/// `previous_assignee_id` is `None` (plain claim guards on unassigned).
pub async fn claim_with_event(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<(ThreadClaimResult, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let lease = ClaimLeaseId::new();
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = $1, assignment_expires_at = NULL, claim_lease_id = $3, claimed_at = NOW(), work_started_at = NULL, updated_at = NOW()
         WHERE id = $2 AND assignee_id IS NULL AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
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

/// Threads in `workspace_id` currently assigned to `member_id` — an agent's
/// work queue. Live threads only, oldest first. Uses `idx_threads_assignee`.
pub async fn list_assigned(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
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

/// Threads under review in `workspace_id` that name `member_id` as a reviewer
/// and do not have that member's approval yet: the reviews waiting on them,
/// oldest first.
pub async fn list_review_requests(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, COALESCE(
                    (SELECT MAX(tt.occurred_at) FROM maidan_thread_transitions tt
                     WHERE tt.thread_id = t.id AND tt.to_state = 'in_review'),
                    t.updated_at) AS updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         -- review_since: when the thread last entered review, not updated_at,
         -- which a claim renewal or a rename bumps. row_to_thread reads it as
         -- updated_at, so the inbox ages the request from when review began.
         JOIN maidan_thread_reviewers rv ON rv.thread_id = t.id AND rv.member_id = $1
         WHERE c.workspace_id = $2 AND t.state = 'in_review' AND t.tombstoned_at IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM maidan_thread_reviews r
             WHERE r.thread_id = t.id AND r.reviewer_id = $1 AND r.decision = 'approve'
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
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_thread).collect()
}

/// A thread `claim_next` took, with the claim it took over when that claim's
/// lease had lapsed (see [`lapsed_claim`]).
type Claimed = (
    Thread,
    Option<(MemberId, DateTime<Utc>, Option<DateTime<Utc>>)>,
);

/// Claim the thread `claim_next` would give `member_id` in `scope`, on the
/// caller's transaction. `FOR UPDATE SKIP LOCKED` is the canonical concurrent
/// work-queue pattern: parallel claimers skip each other's locked candidate
/// and each gets a distinct thread. Shared by both `claim_next` variants and
/// both scopes so they take the same thread.
async fn claim_next_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: ClaimScope,
    member_id: MemberId,
    lease_secs: Option<i64>,
) -> Result<Option<Claimed>, StoreError> {
    let expires = lease_secs.map(|s| chrono::Utc::now() + chrono::Duration::seconds(s));
    let candidate = claim_next::candidate_select(
        scope,
        "cand.id, cand.assignee_id AS prev_assignee,
         cand.assignment_expires_at AS prev_deadline, cand.work_started_at AS prev_started",
        &ClaimSql {
            member: "$1",
            scope: "$2",
            dm_channel: "$5",
            now: "NOW()",
            hours_waiting: "FLOOR(EXTRACT(EPOCH FROM (NOW() - cand.created_at)) / 3600)",
        },
    );
    // The CTE captures the candidate's PRE-update claim (`prev_*`) so a
    // takeover of a lapsed lease can end the dead holder's claim, charged.
    let row = sqlx::query(&format!(
        "WITH next AS ({candidate} FOR UPDATE OF cand SKIP LOCKED)
         UPDATE maidan_threads t SET assignee_id = $1, assignment_expires_at = $3, claim_lease_id = $4, claimed_at = NOW(), work_started_at = NULL, updated_at = NOW()
         FROM next WHERE t.id = next.id
         RETURNING t.id, t.channel_id, t.parent_thread_id, t.title, t.state, t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id, next.prev_assignee, next.prev_deadline, next.prev_started"
    ))
    .bind(member_id.0)
    .bind(scope.id())
    .bind(expires)
    .bind(ClaimLeaseId::new().0)
    .bind(DM_CHANNEL_NAME)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| Ok((row_to_thread(&row)?, lapsed_claim(&row))))
        .transpose()
}

/// Atomically claim the oldest unassigned live thread in `channel_id` for
/// `member_id`. `None` when there is no unassigned work.
pub async fn claim_next(
    pool: &PgPool,
    channel_id: ChannelId,
    member_id: MemberId,
    lease_secs: Option<i64>,
) -> Result<Option<Thread>, StoreError> {
    // Claimable = unassigned OR the lease has expired (dead-agent recovery).
    // The explicit transaction exists so the worker record commits with the
    // claim — and it does not weaken SKIP LOCKED, which already ran inside an
    // implicit transaction of its own.
    let mut tx = pool.begin().await?;
    let Some((thread, lapsed)) = claim_next_in_tx(
        &mut tx,
        ClaimScope::Channel(channel_id),
        member_id,
        lease_secs,
    )
    .await?
    else {
        tx.commit().await?;
        return Ok(None);
    };
    thread_workers::record_in_tx(&mut tx, thread.id, member_id).await?;
    // A takeover of a lapsed lease ends that claim here, charged as the reaper
    // would charge it; this variant returns no events, but the log still
    // records the end.
    if let Some((holder, deadline, started)) = lapsed {
        end_lapsed_claim_in_tx(&mut tx, &thread, holder, deadline, started).await?;
    }
    tx.commit().await?;
    Ok(Some(thread))
}

/// Task-queue depth for a channel — see the SQLite twin. Uses `NOW()` inline
/// (as `claim_next` does) so `ready` matches its claimability predicate
/// exactly.
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
                       AND NOT EXISTS (SELECT 1 FROM maidan_thread_unclaimable u WHERE u.thread_id = t.id)
                       AND NOT EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = t.id)
                       AND NOT EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                     THEN 1 ELSE 0 END), 0) AS ready_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < NOW()))
                       AND NOT EXISTS (SELECT 1 FROM maidan_thread_unclaimable u WHERE u.thread_id = t.id)
                       AND (
                           EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = t.id)
                           OR EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                       )
                     THEN 1 ELSE 0 END), 0) AS blocked_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < NOW()))
                       AND EXISTS (SELECT 1 FROM maidan_thread_unclaimable u WHERE u.thread_id = t.id)
                     THEN 1 ELSE 0 END), 0) AS unclaimable_count
         FROM maidan_threads t
         WHERE t.channel_id = $1
           AND t.state = 'open'
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
        unclaimable: row.get::<i64, _>("unclaimable_count"),
    })
}

/// Channel occupancy — the two-clocks refinement of `channel_queue_depth`.
/// Splits the held threads by the working clock: `claimed` (live lease,
/// `work_started_at` unset) vs `working` (started). `queued` and `blocked`
/// cover the available (unassigned or lease-expired) threads exactly as
/// `queue_depth` does. `NOW()` inline so `queued` matches the claimability
/// predicate. The four sub-counts partition `open`.
pub async fn channel_occupancy(
    pool: &PgPool,
    channel_id: ChannelId,
) -> Result<ChannelOccupancy, StoreError> {
    let row = sqlx::query(
        "SELECT
             COUNT(*) AS open_count,
             COALESCE(SUM(CASE WHEN t.assignee_id IS NOT NULL
                       AND (t.assignment_expires_at IS NULL OR t.assignment_expires_at >= NOW())
                       AND t.work_started_at IS NULL
                     THEN 1 ELSE 0 END), 0) AS claimed_count,
             COALESCE(SUM(CASE WHEN t.assignee_id IS NOT NULL
                       AND (t.assignment_expires_at IS NULL OR t.assignment_expires_at >= NOW())
                       AND t.work_started_at IS NOT NULL
                     THEN 1 ELSE 0 END), 0) AS working_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < NOW()))
                       AND NOT EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = t.id)
                       AND NOT EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                     THEN 1 ELSE 0 END), 0) AS queued_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < NOW()))
                       AND (
                           EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = t.id)
                           OR EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                       )
                     THEN 1 ELSE 0 END), 0) AS blocked_count
         FROM maidan_threads t
         WHERE t.channel_id = $1
           AND t.state = 'open'
           AND t.tombstoned_at IS NULL",
    )
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

/// Atomic claim-next in `scope` + its `ThreadAssignmentChanged` event in one
/// tx. Conditional: the event is appended **only** when a thread was claimed;
/// nothing to claim yields `(None, [])`. `previous_assignee_id` is `None`
/// (behaviour-preserving — matches the old route).
pub(crate) async fn claim_next_with_event(
    pool: &PgPool,
    scope: ClaimScope,
    member_id: MemberId,
    lease_secs: Option<i64>,
) -> Result<(Option<Thread>, Vec<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let Some((thread, lapsed)) = claim_next_in_tx(&mut tx, scope, member_id, lease_secs).await?
    else {
        tx.commit().await?;
        return Ok((None, Vec::new()));
    };
    let mut events = Vec::new();
    // A reclaim of an expired lease: the previous holder's claim ended.
    if let Some((holder, deadline, started)) = lapsed {
        events.push(end_lapsed_claim_in_tx(&mut tx, &thread, holder, deadline, started).await?);
    }
    events.push(append_assignment_event(&mut tx, &thread, member_id, None, None).await?);
    tx.commit().await?;
    Ok((Some(thread), events))
}

/// Reap up to `limit` lapsed leases on open, live threads, charging each
/// claim's worked time to its thread's wall budget and appending a
/// `ClaimExpired` (or, over budget, a `ClaimFailed`) for each dead holder, all
/// in one transaction. `SKIP LOCKED` lets concurrent reapers (one per replica)
/// and `claim_next` run beside it without waiting on, double-reporting or
/// double-charging the same thread.
pub async fn reap_expired_claims(
    pool: &PgPool,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "WITH lapsed AS (
             SELECT id, assignee_id AS prev_assignee, assignment_expires_at AS prev_deadline,
                    work_started_at AS prev_started
             FROM maidan_threads
             WHERE assignee_id IS NOT NULL
               AND assignment_expires_at IS NOT NULL AND assignment_expires_at < $1
               AND tombstoned_at IS NULL AND state = 'open'
             ORDER BY assignment_expires_at ASC, id ASC
             LIMIT $2
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_threads t SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = NOW()
         FROM lapsed WHERE t.id = lapsed.id
         RETURNING t.id, t.channel_id, t.parent_thread_id, t.title, t.state, t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id, lapsed.prev_assignee, lapsed.prev_deadline, lapsed.prev_started",
    )
    .bind(now)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let mut reaped = Vec::with_capacity(rows.len());
    for row in &rows {
        let (holder, deadline, started) = lapsed_claim(row).ok_or_else(|| {
            StoreError::InvalidInput("a reaped claim has no holder or deadline".into())
        })?;
        reaped.push((deadline, row_to_thread(row)?, holder, started));
    }
    reaped.sort_by_key(|(deadline, thread, _, _)| (*deadline, thread.id.0));
    let mut events = Vec::with_capacity(reaped.len());
    for (deadline, thread, holder, started) in &reaped {
        events.push(end_lapsed_claim_in_tx(&mut tx, thread, *holder, *deadline, *started).await?);
    }
    tx.commit().await?;
    Ok(events)
}

/// Report leased claims taken before `claimed_before` that their holder never
/// acknowledged, once per claim. See [`Store::report_unacknowledged_claims`].
pub async fn report_unacknowledged_claims(
    pool: &PgPool,
    now: DateTime<Utc>,
    claimed_before: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "WITH stale AS (
             SELECT id FROM maidan_threads
             WHERE claim_lease_id IS NOT NULL AND work_started_at IS NULL
               AND assignee_id IS NOT NULL
               AND claimed_at IS NOT NULL AND claimed_at < $1
               AND assignment_expires_at IS NOT NULL AND assignment_expires_at >= $2
               AND unacknowledged_lease_id IS DISTINCT FROM claim_lease_id
               AND tombstoned_at IS NULL AND state = 'open'
             ORDER BY claimed_at ASC, id ASC
             LIMIT $3
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_threads t SET unacknowledged_lease_id = t.claim_lease_id
         FROM stale WHERE t.id = stale.id
         RETURNING t.id, t.channel_id, t.parent_thread_id, t.title, t.state, t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id, t.claimed_at",
    )
    .bind(claimed_before)
    .bind(now)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let mut stale = rows
        .iter()
        .map(|row| {
            Ok((
                row.get::<DateTime<Utc>, _>("claimed_at"),
                row_to_thread(row)?,
            ))
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    stale.sort_by_key(|(claimed_at, thread)| (*claimed_at, thread.id.0));
    let mut events = Vec::with_capacity(stale.len());
    for (claimed_at, thread) in &stale {
        events.push(append_claim_unacknowledged_event(&mut tx, thread, *claimed_at).await?);
    }
    tx.commit().await?;
    Ok(events)
}

async fn append_claim_unacknowledged_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
/// `NotFound` if the thread is gone or the caller isn't the holder.
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
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
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

/// Stamp the working clock: the current holder acknowledges the claim and
/// begins work. Fenced by `(assignee_id, claim_lease_id)` — only the live
/// holder presenting the matching token can start the clock. `COALESCE` keeps
/// the first start time, so a re-acknowledge within the same claim epoch is
/// idempotent (a reclaim reset `work_started_at` to NULL, so the next holder
/// stamps fresh). `NotFound` if the thread is gone, the caller isn't the
/// holder, or the token is stale.
pub async fn acknowledge_claim(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET work_started_at = COALESCE(work_started_at, NOW()), updated_at = NOW()
         WHERE id = $1 AND assignee_id = $2 AND claim_lease_id = $3 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Release a claim (graceful handoff): the current holder returns the thread to
/// the queue immediately instead of waiting for the lease to lapse — e.g. an
/// agent shutting down on SIGTERM. Fenced by `(assignee_id, claim_lease_id)`;
/// clears the assignee, lease, and working clock in one write. `NotFound` if
/// the caller isn't the holder or the token is stale.
pub async fn release_claim(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<Thread, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = NOW()
         WHERE id = $1 AND assignee_id = $2 AND claim_lease_id = $3 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_thread(&row)
}

/// Release a claim and append its `ThreadAssignmentChanged` event in one tx.
/// The previous assignee is the caller (the fence guarantees it). `NotFound` if
/// the caller isn't the holder.
pub async fn release_claim_with_event(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
) -> Result<(Thread, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "UPDATE maidan_threads SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = NOW()
         WHERE id = $1 AND assignee_id = $2 AND claim_lease_id = $3 AND tombstoned_at IS NULL
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
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
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
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
/// ascending. `after` is an exclusive cursor (the prior page's last thread id);
/// `None` starts from the beginning. The channel-scoped twin of
/// [`page_for_workspace`] — bounds the previously-unbounded channel thread
/// list.
pub async fn page_for_channel(
    pool: &PgPool,
    channel_id: ChannelId,
    after: Option<ThreadId>,
    limit: i64,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.channel_id, t.parent_thread_id, t.title, t.state,
                t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id
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

/// Enforce the workspace's spawn budget when creating a CHILD thread: refuse
/// once the parent already holds `max_children` children, or once its nesting
/// would exceed `max_depth`. Root threads (no parent) and workspaces with no
/// budget are unrestricted. A refusal is a typed `SpawnRejected` → REST 409 /
/// MCP InvalidParams, and it carries the payload the route publishes as
/// `ThreadSpawnDenied`. Coordination cost is n(n-1)/2 — do not admit a further
/// agent onto a late claim.
async fn enforce_spawn_budget(
    pool: &PgPool,
    channel_id: ChannelId,
    parent_thread_id: Option<ThreadId>,
) -> Result<(), StoreError> {
    let Some(parent_id) = parent_thread_id else {
        return Ok(());
    };
    let Some(ws_row) = sqlx::query("SELECT workspace_id FROM maidan_channels WHERE id = $1")
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

pub(super) fn row_to_thread(row: &sqlx::postgres::PgRow) -> Result<Thread, StoreError> {
    let state_str: String = row.get("state");
    let state = parse_state(&state_str)?;
    let parent: Option<Uuid> = row.get("parent_thread_id");
    let assignee: Option<Uuid> = row.get("assignee_id");
    let owner: Option<Uuid> = row.get("owner_id");
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
        owner_id: owner.map(MemberId),
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
