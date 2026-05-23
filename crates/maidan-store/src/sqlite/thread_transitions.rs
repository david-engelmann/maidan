use chrono::Utc;
use maidan_fsm::ThreadAction;
use maidan_types::{MemberId, ThreadId, ThreadTransitionResult};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::threads::row_to_thread;
use crate::error::StoreError;

pub async fn transition(
    pool: &SqlitePool,
    thread_id: ThreadId,
    actor_id: MemberId,
    action: ThreadAction,
) -> Result<ThreadTransitionResult, StoreError> {
    let mut tx = pool.begin().await?;

    let row = sqlx::query(
        "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at
         FROM maidan_threads WHERE id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
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
            "SELECT id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at
             FROM maidan_threads WHERE id = ?",
        )
        .bind(parent_id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        let parent = row_to_thread(&parent_row)?;
        maidan_fsm::hsm::parent_allows_transition(parent.state, to_state)
            .map_err(|e| StoreError::Conflict(e.as_str().into()))?;
    }

    let transition_id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO maidan_thread_transitions
            (id, thread_id, from_state, to_state, actor_id, occurred_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(transition_id)
    .bind(thread_id.0)
    .bind(from_state.as_str())
    .bind(to_state.as_str())
    .bind(actor_id.0)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    let row = sqlx::query(
        "UPDATE maidan_threads SET state = ?, updated_at = ?
         WHERE id = ?
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at",
    )
    .bind(to_state.as_str())
    .bind(&now)
    .bind(thread_id.0)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let thread = row_to_thread(&row)?;
    Ok(ThreadTransitionResult {
        thread,
        from_state,
        to_state,
    })
}
