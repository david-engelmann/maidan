use chrono::Utc;
use maidan_fsm::ThreadAction;
use maidan_types::{
    Event, MemberId, StoredEvent, ThreadId, ThreadState, ThreadTransition, ThreadTransitionResult,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::threads::row_to_thread;
use crate::error::StoreError;
use crate::postgres::events;

/// Cluster 375 (Wave 2 #22): gate a `closed` transition. Refuses close until the
/// review requirement is met — `k` distinct **qualifying** approvals (decision =
/// approve, reviewer is neither owner nor assignee, and, when a named reviewer
/// set exists, is in it) — and no unresolved `refutes` reference targets the
/// thread. Runs on the transition's own tx so the check can't be raced.
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
              WHERE r.thread_id = $1 AND r.decision = 'approve'
                AND (t.owner_id IS NULL OR r.reviewer_id <> t.owner_id)
                AND (t.assignee_id IS NULL OR r.reviewer_id <> t.assignee_id)
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
    if required > 0 && approvals < required {
        return Err(StoreError::Conflict(format!(
            "review requirement not met: {approvals} of {required} required approvals"
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

/// The FSM transition on a caller-supplied tx, without committing (Cluster 208).
/// Shared by `transition` (commit only) and `transition_with_event` (append the
/// `ThreadStateChanged` event in the same tx, then commit).
async fn transition_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    actor_id: MemberId,
    action: ThreadAction,
) -> Result<ThreadTransitionResult, StoreError> {
    let row = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id
         FROM maidan_threads WHERE id = $1",
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

    // Separation of duties (Cluster 355, W1): on an owner-governed thread the
    // claimer cannot land its own work — a terminal transition (the "merge") must
    // be performed by the owner or another member. Un-owned threads are
    // unrestricted, so this is inert until an owner is set.
    if thread.owner_id.is_some() && to_state.is_terminal() && thread.assignee_id == Some(actor_id) {
        return Err(StoreError::Conflict(
            "separation of duties: the claimer cannot land its own work on an owned thread; the owner or another member must perform this transition".into(),
        ));
    }

    // Required reviewers (Cluster 375, Wave 2 #22): a `closed` transition is gated
    // on k qualifying approvals + no unresolved `refutes` edge.
    if to_state == ThreadState::Closed {
        review_gate_in_tx(tx, thread_id).await?;
    }

    if let Some(parent_id) = thread.parent_thread_id {
        let parent_row = sqlx::query(
            "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id
             FROM maidan_threads WHERE id = $1",
        )
        .bind(parent_id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        let parent = row_to_thread(&parent_row)?;
        maidan_fsm::hsm::parent_allows_transition(parent.state, to_state)
            .map_err(|e| StoreError::Conflict(e.as_str().into()))?;
    }

    let transition_id = Uuid::new_v4();
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

    let row = sqlx::query(
        "UPDATE maidan_threads SET state = $1, updated_at = $2
         WHERE id = $3
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(to_state.as_str())
    .bind(now)
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;

    let thread = row_to_thread(&row)?;
    Ok(ThreadTransitionResult {
        thread,
        from_state,
        to_state,
    })
}

pub async fn transition(
    pool: &PgPool,
    thread_id: ThreadId,
    actor_id: MemberId,
    action: ThreadAction,
) -> Result<ThreadTransitionResult, StoreError> {
    let mut tx = pool.begin().await?;
    let result = transition_in_tx(&mut tx, thread_id, actor_id, action).await?;
    tx.commit().await?;
    Ok(result)
}

/// Transition a thread's state and append its `ThreadStateChanged` event in one
/// transaction (Cluster 208 transactional outbox).
pub async fn transition_with_event(
    pool: &PgPool,
    thread_id: ThreadId,
    actor_id: MemberId,
    action: ThreadAction,
) -> Result<(ThreadTransitionResult, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let result = transition_in_tx(&mut tx, thread_id, actor_id, action).await?;
    let (workspace_id, channel_id) = events::thread_scope_in_tx(&mut tx, thread_id).await?;
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
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((result, stored))
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
         ORDER BY occurred_at ASC
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
