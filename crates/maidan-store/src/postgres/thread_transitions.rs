use chrono::Utc;
use maidan_fsm::ThreadAction;
use maidan_types::{
    CloseAbandoned, CloseReason, ContentKeyring, Event, MemberId, NewAuditEvent, RelationKind,
    StoredEvent, Thread, ThreadId, ThreadState, ThreadTransition, ThreadTransitionResult,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::threads::row_to_thread;
use crate::error::StoreError;
use crate::postgres::events;

/// Gate a `closed` transition. Refuses close until the review requirement is
/// met — `k` distinct **qualifying** approvals (decision = approve, reviewer is
/// neither owner nor assignee, and, when a named reviewer set exists, is in it)
/// — and no unresolved `refutes` reference targets the thread. Runs on the
/// transition's own tx so the check can't be raced.
async fn review_gate_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    let row = sqlx::query(
        "SELECT
           COALESCE((SELECT required_count FROM maidan_thread_review_reqs WHERE thread_id = $1), 0)
             AS required_count,
           (SELECT COUNT(*) FROM maidan_thread_reviews r
              JOIN maidan_threads t ON t.id = r.thread_id
              JOIN maidan_channels c ON c.id = t.channel_id
              -- A removed member's approval no longer counts, nor one from
              -- another workspace.
              JOIN maidan_members m ON m.id = r.reviewer_id
                AND m.workspace_id = c.workspace_id AND m.tombstoned_at IS NULL
              WHERE r.thread_id = $1 AND r.decision = 'approve' AND r.dismissed_at IS NULL
                -- Bound to what the current review was handed.
                -- A thread never handed over has no packet to bind to; once it has
                -- one, an approval given before or for another hand-off counts no more.
                AND (
                  NOT EXISTS (SELECT 1 FROM maidan_review_packets p WHERE p.thread_id = r.thread_id)
                  OR r.evidence_root = (
                    SELECT p.evidence_root FROM maidan_review_packets p
                    WHERE p.thread_id = r.thread_id
                    ORDER BY p.created_at DESC, p.id DESC LIMIT 1
                  )
                )
                AND (t.owner_id IS NULL OR r.reviewer_id <> t.owner_id)
                AND (t.assignee_id IS NULL OR r.reviewer_id <> t.assignee_id)
                -- And never held it. `assignee_id` is the live
                -- holder, which a release clears — so on its own it let an
                -- implementer release the claim and approve their own work.
                AND NOT EXISTS (
                  SELECT 1 FROM maidan_thread_workers w
                  WHERE w.thread_id = r.thread_id AND w.member_id = r.reviewer_id
                )
                -- Nor may whoever actually submitted it: a delegate that owns or
                -- worked the thread cannot approve it with a reviewer's borrowed token.
                AND (r.actor_id IS NULL OR (
                  EXISTS (SELECT 1 FROM maidan_members ma
                          WHERE ma.id = r.actor_id AND ma.tombstoned_at IS NULL
                            AND ma.workspace_id = c.workspace_id)
                  AND (t.owner_id IS NULL OR r.actor_id <> t.owner_id)
                  AND (t.assignee_id IS NULL OR r.actor_id <> t.assignee_id)
                  AND NOT EXISTS (
                    SELECT 1 FROM maidan_thread_workers wa
                    WHERE wa.thread_id = r.thread_id AND wa.member_id = r.actor_id
                  )
                ))
                AND (NOT EXISTS (SELECT 1 FROM maidan_thread_reviewers rv WHERE rv.thread_id = $1)
                     OR EXISTS (SELECT 1 FROM maidan_thread_reviewers rv
                                WHERE rv.thread_id = $1 AND rv.member_id = r.reviewer_id))
           ) AS approvals",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    let required: i64 = row.get("required_count");
    let approvals: i64 = row.get("approvals");
    if required > 0 {
        // Approvals count only for what they were shown: content changed since
        // the hand-off makes them approvals of something else.
        if let Some(root) = super::review_packets::latest_root_in_tx(tx, thread_id).await? {
            super::review_packets::ensure_unchanged_in_tx(tx, thread_id, &root).await?;
        }
    }
    if required > 0 && approvals < required {
        return Err(StoreError::Conflict(format!(
            "review requirement not met: {approvals} of {required} required approvals. \
             Next: a reviewer who did not work on this thread approves it \
             (submit_review, decision approve), then close it again; \
             closing before that is refused the same way"
        )));
    }
    let refuted = sqlx::query(
        "SELECT 1 FROM maidan_references
         WHERE relation = 'refutes' AND dst_kind = 'thread' AND dst_id = $1 LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .is_some();
    if refuted {
        return Err(StoreError::Conflict(
            "a `refutes` reference blocks close until it is resolved".into(),
        ));
    }
    Ok(())
}

/// Gate a `start_review` on a thread whose close needs approvals: a reviewer
/// is asked to approve something, so a result must be posted first. A thread
/// with no review requirement may go to review without one; the approval card
/// says so instead (decided 2026-10-04).
async fn result_gate_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    let row = sqlx::query(
        "SELECT
           COALESCE((SELECT required_count FROM maidan_thread_review_reqs WHERE thread_id = $1), 0)
             AS required_count,
           (SELECT COUNT(*) FROM maidan_thread_results WHERE thread_id = $1) AS results",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    let required: i64 = sqlx::Row::get(&row, "required_count");
    let results: i64 = sqlx::Row::get(&row, "results");
    if required > 0 && results == 0 {
        return Err(StoreError::Conflict(
            "no result posted: this thread needs approval to close, so a reviewer must have \
             something to approve. Next: post the result (set_thread_result, or \
             PUT /threads/{id}/result), then start_review again"
                .into(),
        ));
    }
    Ok(())
}

/// Whether the thread holds an approval from an outside reviewer: a member of
/// the thread's own workspace who is not the owner, never held the thread, and
/// was not acting for anyone who did, bound to the current review packet; or a
/// qualifying green land-gate pass from such a member. Only such an approval
/// lets a past holder land owned work (#1569).
async fn outside_approval_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 AS hit FROM maidan_thread_reviews r
           JOIN maidan_threads t ON t.id = r.thread_id
           JOIN maidan_channels c ON c.id = t.channel_id
           JOIN maidan_members m ON m.id = r.reviewer_id
          WHERE r.thread_id = $1 AND r.decision = 'approve' AND r.dismissed_at IS NULL
            AND m.workspace_id = c.workspace_id AND m.tombstoned_at IS NULL
            AND (t.owner_id IS NULL OR r.reviewer_id <> t.owner_id)
            AND NOT EXISTS (SELECT 1 FROM maidan_thread_workers w
                            WHERE w.thread_id = r.thread_id AND w.member_id = r.reviewer_id)
            AND (r.actor_id IS NULL OR (
              EXISTS (SELECT 1 FROM maidan_members ma
                      WHERE ma.id = r.actor_id AND ma.tombstoned_at IS NULL
                        AND ma.workspace_id = c.workspace_id)
              AND (t.owner_id IS NULL OR r.actor_id <> t.owner_id)
              AND NOT EXISTS (SELECT 1 FROM maidan_thread_workers wa
                              WHERE wa.thread_id = r.thread_id AND wa.member_id = r.actor_id)))
            AND (
              NOT EXISTS (SELECT 1 FROM maidan_review_packets p WHERE p.thread_id = r.thread_id)
              OR r.evidence_root = (
                SELECT p.evidence_root FROM maidan_review_packets p
                WHERE p.thread_id = r.thread_id
                ORDER BY p.created_at DESC, p.id DESC LIMIT 1))
          LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    if row.is_some() {
        return Ok(true);
    }
    // A qualifying green land-gate pass is the same outside verification:
    // recorded by a skilled member who is neither the owner nor ever a holder,
    // and who is still a member: a removed recorder's pass no longer counts.
    let crate::attestation::GateReading::Recorded(pass) =
        super::land_gate::read_in_tx(tx, thread_id).await?
    else {
        return Ok(false);
    };
    if !pass.qualifies() {
        return Ok(false);
    }
    let same_workspace = sqlx::query(
        "SELECT 1 AS hit FROM maidan_threads t
           JOIN maidan_channels c ON c.id = t.channel_id
           JOIN maidan_members m ON m.workspace_id = c.workspace_id
          WHERE m.tombstoned_at IS NULL AND t.id = $1 AND m.id = $2",
    )
    .bind(thread_id.0)
    .bind(pass.recorded_by.0)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(same_workspace.is_some())
}

/// The FSM transition on a caller-supplied tx, without committing. Shared by
/// `transition` (commit only) and `transition_with_event` (append the
/// `ThreadStateChanged` event in the same tx, then commit).
/// [`transition_in_spawning`] for callers that publish nothing of their own.
pub(crate) async fn transition_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    actor_id: MemberId,
    action: ThreadAction,
) -> Result<ThreadTransitionResult, StoreError> {
    Ok(transition_in_spawning(tx, thread_id, actor_id, action)
        .await?
        .0)
}

/// Make the transition in `tx`, returning with it the events of any
/// review child it spawned, for the caller to publish after commit.
pub(crate) async fn transition_in_spawning(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    actor_id: MemberId,
    action: ThreadAction,
) -> Result<(ThreadTransitionResult, Vec<StoredEvent>), StoreError> {
    let mut spawned = Vec::new();
    let row = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, created_by
         FROM maidan_threads WHERE id = $1 FOR UPDATE",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)?;

    let thread = row_to_thread(&row)?;
    if thread.tombstoned_at.is_some() {
        return Err(StoreError::NotFound);
    }

    let from_state = thread.state;
    let to_state = maidan_fsm::apply(from_state, action).map_err(|invalid| {
        StoreError::Conflict(format!(
            "invalid transition from {} via {}",
            invalid.from.as_str(),
            invalid.action.as_str()
        ))
    })?;

    // Separation of duties: on an owner-governed thread the claimer cannot land
    // its own work — a terminal transition (the "merge") must be performed by
    // the owner or another member. Un-owned threads are unrestricted, so this
    // is inert until an owner is set. The claimer is anyone who ever held the
    // thread, not only the live holder: a release or a reaped claim clears
    // `assignee_id`, and the work is still theirs (#1569). A past holder may
    // still land it once an outside reviewer has approved: the rule stops a
    // holder standing in for an independent review, not landing after one.
    separation_of_duties_in_tx(tx, &thread, actor_id, to_state).await?;

    // Required reviewers: a `closed` transition is gated on k qualifying
    // approvals + no unresolved `refutes` edge. LandGate pointer: a `closed`
    // transition is also gated on a qualifying green pass when a
    // pointer/requirement exists.
    if action == ThreadAction::StartReview {
        result_gate_in_tx(tx, thread_id).await?;
        // What the reviewers are handed, pinned before the state moves.
        super::review_packets::record_in_tx(tx, thread_id, actor_id).await?;
        // A change entering review gets its second-family review child.
        spawned = super::review_family::spawn_in_tx(tx, thread_id, actor_id).await?;
    }
    if to_state == ThreadState::Closed {
        review_gate_in_tx(tx, thread_id).await?;
        super::land_gate::gate_in_tx(tx, thread_id).await?;
    }

    land_in_tx(tx, &thread, actor_id, to_state).await
}

/// The separation-of-duties rule described at its call in [`transition_in_tx`].
async fn separation_of_duties_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread: &Thread,
    actor_id: MemberId,
    to_state: ThreadState,
) -> Result<(), StoreError> {
    let thread_id = thread.id;
    if thread.owner_id.is_some()
        && to_state.is_terminal()
        && (thread.assignee_id == Some(actor_id)
            || (super::thread_workers::has_worked_in_tx(tx, thread_id, actor_id).await?
                && !outside_approval_in_tx(tx, thread_id).await?))
    {
        return Err(StoreError::Conflict(
            "separation of duties: the claimer cannot land its own work on an owned thread; the owner or another member must perform this transition".into(),
        ));
    }
    Ok(())
}

/// Move `thread` to `to_state` once every gate has passed: the parent's
/// state check, the transition row, and on a terminal state the claim
/// charged and cleared.
async fn land_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread: &Thread,
    actor_id: MemberId,
    to_state: ThreadState,
) -> Result<ThreadTransitionResult, StoreError> {
    let thread_id = thread.id;
    let from_state = thread.state;
    if let Some(parent_id) = thread.parent_thread_id {
        let parent_row = sqlx::query(
            "SELECT id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, created_by
             FROM maidan_threads WHERE id = $1",
        )
        .bind(parent_id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        let parent = row_to_thread(&parent_row)?;
        // A plan's children land while the plan stays open under its
        // planner's claim, and the plan closes after them (#1421), so only
        // an archived plan holds a child back. A review child the review family
        // opened finishes before the change it reviews merges, so it is not
        // held back either; a child merely labelled a review is.
        let child_to = if super::plans::is_child_in_tx(tx, thread_id).await?
            || super::thread_attributes::is_review_in_tx(tx, thread_id).await?
        {
            ThreadState::Open
        } else {
            to_state
        };
        maidan_fsm::hsm::parent_allows_transition(parent.state, child_to)
            .map_err(|e| StoreError::Conflict(e.as_str().into()))?;
    }

    let transition_id = Uuid::now_v7();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO maidan_thread_transitions
            (id, thread_id, from_state, to_state, actor_id, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(transition_id)
    .bind(thread_id.0)
    .bind(from_state.as_str())
    .bind(to_state.as_str())
    .bind(actor_id.0)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    // Closing (or archiving) ends the claim. Charge the time it worked, then
    // clear it in the same write. A non-terminal transition leaves the claim.
    let terminal = to_state.is_terminal();
    if terminal {
        super::threads::charge_open_claim_in_tx(tx, thread_id).await?;
    }
    let row = sqlx::query(
        "UPDATE maidan_threads SET state = $1, updated_at = $2,
            assignee_id = CASE WHEN $4::boolean THEN NULL ELSE assignee_id END,
            assignment_expires_at = CASE WHEN $4::boolean THEN NULL ELSE assignment_expires_at END,
            claim_lease_id = CASE WHEN $4::boolean THEN NULL ELSE claim_lease_id END,
            claimed_at = CASE WHEN $4::boolean THEN NULL ELSE claimed_at END,
            work_started_at = CASE WHEN $4::boolean THEN NULL ELSE work_started_at END
         WHERE id = $3
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, created_by",
    )
    .bind(to_state.as_str())
    .bind(now)
    .bind(thread_id.0)
    .bind(terminal)
    .fetch_one(&mut **tx)
    .await?;

    let thread = row_to_thread(&row)?;
    Ok((
        ThreadTransitionResult {
            thread,
            from_state,
            to_state,
        },
        spawned,
    ))
}

/// Close a thread that will not land (`close-abandoned`), in one transaction.
///
/// The thread must be `open` or `in_review`; it moves straight to `closed`.
/// The caller has already checked who may do this (the producer, the claim
/// holder, or a maintainer); `may_close` re-checks producer and holder on the
/// locked row so a claim that moved in between is still judged correctly.
/// A thread with a result keeps every landing gate: separation of duties,
/// required reviews and the land gate. A thread with no result skips them,
/// since there is nothing to review.
///
/// `duplicate_of` must be a live thread in the same workspace that `actor_id`
/// can read, and not the thread itself; any other id gets the same refusal.
/// A duplicate records `thread duplicates duplicate_of`; a superseded thread
/// records `duplicate_of supersedes thread`.
///
/// Returns the transition and the events in order: the reference (when one
/// was recorded), the state change, then whatever the close owed a plan.
#[allow(clippy::too_many_arguments)]
pub async fn close_abandoned(
    pool: &PgPool,
    keys: &ContentKeyring,
    thread_id: ThreadId,
    actor_id: MemberId,
    maintainer: bool,
    close: &CloseAbandoned,
    mut audit: NewAuditEvent,
) -> Result<(ThreadTransitionResult, Vec<StoredEvent>), StoreError> {
    use crate::thread_access::readable_thread;
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, created_by
         FROM maidan_threads WHERE id = $1 FOR UPDATE",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let thread = row_to_thread(&row)?;
    if thread.tombstoned_at.is_some() {
        return Err(StoreError::NotFound);
    }
    if !matches!(thread.state, ThreadState::Open | ThreadState::InReview) {
        return Err(StoreError::Conflict(format!(
            "only an open or in-review thread can be closed as abandoned; this one is {}",
            thread.state.as_str()
        )));
    }
    let producer = thread.created_by == Some(actor_id);
    let holder = thread.assignee_id == Some(actor_id);
    if !(producer || holder || maintainer) {
        return Err(StoreError::Conflict(
            "only the thread's producer, its claim holder, or a maintainer can close it as abandoned".into(),
        ));
    }

    let relation = match (close.reason, close.duplicate_of) {
        (CloseReason::Abandoned, Some(_)) => {
            return Err(StoreError::InvalidInput(
                "duplicate_of applies to a duplicate or a superseded thread, not an abandoned one"
                    .into(),
            ))
        }
        (CloseReason::Duplicate, None) => {
            return Err(StoreError::InvalidInput(
                "a duplicate must name the thread it duplicates in duplicate_of".into(),
            ))
        }
        (_, Some(other)) if other == thread_id => {
            return Err(StoreError::InvalidInput(
                "duplicate_of cannot be the thread being closed".into(),
            ))
        }
        (CloseReason::Duplicate, Some(other)) => Some((thread_id, other, RelationKind::Duplicates)),
        (CloseReason::Superseded, Some(other)) => {
            Some((other, thread_id, RelationKind::Supersedes))
        }
        (_, None) => None,
    };

    let mut stored = Vec::new();
    if let Some((src, dst, kind)) = relation {
        let other = if src == thread_id { dst } else { src };
        let (ws, _) = events::thread_scope_in_tx(&mut tx, thread_id).await?;
        let readable = readable_thread("t.id", "$2", "$3::uuid", "$4");
        let sql = format!(
            "SELECT 1 AS one FROM maidan_threads t JOIN maidan_channels c ON c.id = t.channel_id
         WHERE t.id = $1 AND c.workspace_id = $2 AND t.tombstoned_at IS NULL AND {readable}"
        );
        let found = sqlx::query(&sql)
            .bind(other.0)
            .bind(ws.0)
            .bind(actor_id.0)
            .bind(maidan_types::DM_CHANNEL_NAME)
            .fetch_optional(&mut *tx)
            .await?;
        if found.is_none() {
            return Err(StoreError::InvalidInput(
                "duplicate_of is not a thread you can read in this workspace".into(),
            ));
        }
        let inserted = sqlx::query(
            "INSERT INTO maidan_references (id, src_kind, src_id, dst_kind, dst_id, relation)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT DO NOTHING
         RETURNING id, src_kind, src_id, dst_kind, dst_id, relation, created_at",
        )
        .bind(Uuid::now_v7())
        .bind(maidan_types::RefSide::Thread.as_str())
        .bind(src.0)
        .bind(maidan_types::RefSide::Thread.as_str())
        .bind(dst.0)
        .bind(kind.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = inserted {
            let reference = super::refs::row_to_reference(&row)?;
            let event = Event::ReferenceAdded {
                occurred_at: Utc::now(),
                reference,
            };
            stored.push(events::append_in_tx(&mut tx, &event).await?);
        }
    }

    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM maidan_thread_results WHERE thread_id = $1) AS had",
    )
    .bind(thread_id.0)
    .fetch_one(&mut *tx)
    .await?;
    let had_result: bool = sqlx::Row::get(&row, "had");
    if had_result {
        separation_of_duties_in_tx(&mut tx, &thread, actor_id, ThreadState::Closed).await?;
        review_gate_in_tx(&mut tx, thread_id).await?;
        super::land_gate::gate_in_tx(&mut tx, thread_id).await?;
    }

    let result = land_in_tx(&mut tx, &thread, actor_id, ThreadState::Closed).await?;
    // Nothing landed, so the dependents stay waiting: a dependency is met
    // only when it is closed or archived without a row here.
    sqlx::query(
        "INSERT INTO maidan_thread_unlanded_closes (thread_id, reason, closed_by)
         VALUES ($1, $2, $3)",
    )
    .bind(thread_id.0)
    .bind(close.reason.as_str())
    .bind(actor_id.0)
    .execute(&mut *tx)
    .await?;
    let mut metadata = match std::mem::take(&mut audit.metadata) {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    metadata.insert("reason".into(), close.reason.as_str().into());
    metadata.insert(
        "duplicate_of".into(),
        close
            .duplicate_of
            .map_or(serde_json::Value::Null, |d| d.0.to_string().into()),
    );
    metadata.insert("had_result".into(), had_result.into());
    metadata.insert("from_state".into(), result.from_state.as_str().into());
    audit.metadata = serde_json::Value::Object(metadata);
    super::audit::append_scoped(
        &mut tx,
        crate::audit_scope::AuditOwner::Thread(thread_id),
        audit,
    )
    .await?;
    stored.push(state_changed_in_tx(&mut tx, actor_id, &result).await?);
    stored.extend(super::plans::after_close_in_tx(&mut tx, keys, thread_id, actor_id).await?);
    tx.commit().await?;
    Ok((result, stored))
}

pub async fn transition(
    pool: &PgPool,
    keys: &ContentKeyring,
    thread_id: ThreadId,
    actor_id: MemberId,
    action: ThreadAction,
) -> Result<ThreadTransitionResult, StoreError> {
    let mut tx = pool.begin().await?;
    let result = transition_in_tx(&mut tx, thread_id, actor_id, action).await?;
    if result.to_state == ThreadState::Closed {
        // Recorded in the event log; this path publishes nothing, its own
        // state change included.
        super::plans::after_close_in_tx(&mut tx, keys, thread_id, actor_id).await?;
    }
    tx.commit().await?;
    Ok(result)
}

/// Transition a thread's state and append its `ThreadStateChanged` event in one
/// transaction. The events come back in order, the state change first, then
/// whatever the close owed a plan around the thread.
pub async fn transition_with_event(
    pool: &PgPool,
    keys: &ContentKeyring,
    thread_id: ThreadId,
    actor_id: MemberId,
    action: ThreadAction,
) -> Result<(ThreadTransitionResult, Vec<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let (result, spawned) = transition_in_spawning(&mut tx, thread_id, actor_id, action).await?;
    let mut stored = vec![state_changed_in_tx(&mut tx, actor_id, &result).await?];
    stored.extend(spawned);
    if result.to_state == ThreadState::Closed {
        stored.extend(super::plans::after_close_in_tx(&mut tx, keys, thread_id, actor_id).await?);
    }
    tx.commit().await?;
    Ok((result, stored))
}

/// Append the `ThreadStateChanged` event for a transition made in `tx`.
pub(crate) async fn state_changed_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor_id: MemberId,
    result: &ThreadTransitionResult,
) -> Result<StoredEvent, StoreError> {
    let thread_id = result.thread.id;
    let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, thread_id).await?;
    let event = Event::ThreadStateChanged {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id,
        actor_id,
        from_state: result.from_state,
        to_state: result.to_state,
        thread: result.thread.clone(),
    };
    events::append_in_tx(tx, &event).await
}

pub async fn list(
    pool: &PgPool,
    thread_id: ThreadId,
    limit: i64,
) -> Result<Vec<ThreadTransition>, StoreError> {
    use chrono::{DateTime, Utc};
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT id, thread_id, from_state, to_state, actor_id, occurred_at
         FROM maidan_thread_transitions
         WHERE thread_id = $1
         ORDER BY occurred_at ASC, id ASC
         LIMIT $2",
    )
    .bind(thread_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(|row| {
            let from_state = parse_state(row.get::<&str, _>("from_state"))?;
            let to_state = parse_state(row.get::<&str, _>("to_state"))?;
            Ok(ThreadTransition {
                id: row.get("id"),
                thread_id: ThreadId(row.get("thread_id")),
                from_state,
                to_state,
                actor_id: MemberId(row.get("actor_id")),
                occurred_at: row.get::<DateTime<Utc>, _>("occurred_at"),
            })
        })
        .collect()
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
