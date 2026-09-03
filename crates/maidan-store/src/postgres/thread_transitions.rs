use chrono::Utc;
use maidan_fsm::ThreadAction;
use maidan_types::{
    Event, MemberId, StoredEvent, ThreadId, ThreadState, ThreadTransition, ThreadTransitionResult,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::threads::row_to_thread;
use crate::error::StoreError;
use crate::postgres::events;

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
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at
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

    if let Some(parent_id) = thread.parent_thread_id {
        let parent_row = sqlx::query(
            "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at
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
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at",
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
