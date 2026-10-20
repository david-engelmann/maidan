//! Cancel a thread; see [`maidan_types::CancelOutcome`].

use chrono::{DateTime, Utc};
use maidan_types::{
    CancelAuthority, CancelOutcome, CancelledThread, ContentKeyring, Event, MemberId, StoredEvent,
    ThreadCancellation, ThreadId, ThreadState, ThreadTransitionResult,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::threads::row_to_thread;
use super::{events, messages, thread_transitions, threads};
use crate::cancel::{notice_body, notice_metadata};
use crate::error::StoreError;

const THREAD_COLS: &str = "id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, created_by";

pub async fn cancel(
    pool: &SqlitePool,
    keys: &ContentKeyring,
    thread_id: ThreadId,
    actor: MemberId,
    reason: &str,
    authority: CancelAuthority,
) -> Result<CancelOutcome, StoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "SELECT {THREAD_COLS} FROM maidan_threads WHERE id = ? AND tombstoned_at IS NULL"
    ))
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let thread = row_to_thread(&row)?;
    if authority == CancelAuthority::Creator && thread.created_by != Some(actor) {
        return Ok(CancelOutcome::NotCreator);
    }
    if let Some(cancellation) = get_in_tx(&mut tx, thread_id).await? {
        return Ok(CancelOutcome::AlreadyCancelled(CancelledThread {
            thread,
            cancellation,
        }));
    }
    if thread.state.is_terminal() {
        return Ok(CancelOutcome::AlreadyFinished(thread.state));
    }

    let now = Utc::now();
    let stamp = now.to_rfc3339();
    // Straight to closed, around the transition gates: those guard landing
    // work (reviews, a green pass, a parent ahead of its child), and a cancel
    // lands nothing.
    sqlx::query(
        "INSERT INTO maidan_thread_transitions
            (id, thread_id, from_state, to_state, actor_id, occurred_at)
         VALUES (?, ?, ?, 'closed', ?, ?)",
    )
    .bind(Uuid::now_v7())
    .bind(thread_id.0)
    .bind(thread.state.as_str())
    .bind(actor.0)
    .bind(&stamp)
    .execute(&mut *tx)
    .await?;
    threads::charge_open_claim_in_tx(&mut tx, thread_id).await?;
    let row = sqlx::query(&format!(
        "UPDATE maidan_threads SET state = 'closed', updated_at = ?,
            assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL,
            claimed_at = NULL, work_started_at = NULL
         WHERE id = ?
         RETURNING {THREAD_COLS}"
    ))
    .bind(&stamp)
    .bind(thread_id.0)
    .fetch_one(&mut *tx)
    .await?;
    let closed = row_to_thread(&row)?;
    sqlx::query(
        "INSERT INTO maidan_thread_unclaimable (thread_id, reason, marked_by, marked_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             reason = excluded.reason, marked_by = excluded.marked_by, marked_at = excluded.marked_at",
    )
    .bind(thread_id.0)
    .bind(reason)
    .bind(actor.0)
    .bind(&stamp)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO maidan_thread_cancellations (thread_id, reason, cancelled_by, cancelled_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(thread_id.0)
    .bind(reason)
    .bind(actor.0)
    .bind(&stamp)
    .execute(&mut *tx)
    .await?;

    let mut stored: Vec<StoredEvent> = Vec::with_capacity(3);
    let transition = ThreadTransitionResult {
        thread: closed.clone(),
        from_state: thread.state,
        to_state: ThreadState::Closed,
    };
    stored.push(thread_transitions::state_changed_in_tx(&mut tx, actor, &transition).await?);
    let (workspace_id, channel_id) = events::thread_scope_in_tx(&mut tx, thread_id).await?;
    let cancelled = Event::ThreadCancelled {
        occurred_at: now,
        workspace_id,
        channel_id,
        thread_id,
        reason: reason.to_string(),
        cancelled_by: actor,
    };
    stored.push(events::append_in_tx(&mut tx, &cancelled).await?);
    let message_row = sqlx::query(
        "INSERT INTO maidan_messages (id, thread_id, author_id, body, metadata, content, posted_at)
         VALUES (?, ?, ?, ?, ?, NULL, ?)
         RETURNING id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at",
    )
    .bind(Uuid::now_v7())
    .bind(thread_id.0)
    .bind(actor.0)
    .bind(notice_body(reason))
    .bind(serde_json::to_string(&notice_metadata(reason))?)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    let message = messages::row_to_message(&message_row)?;
    let posted = Event::MessagePosted {
        occurred_at: now,
        workspace_id,
        channel_id,
        thread_id,
        dm_conversation_id: None,
        message,
        sealed: None,
    };
    stored.push(events::append_with_keys_in_tx(&mut tx, Some(keys), &posted, None).await?);
    // A cancel lands nothing, so a cancelled plan child blocks its dependents.
    stored.extend(super::plans::after_close_in_tx(&mut tx, keys, thread_id, actor).await?);
    tx.commit().await?;
    Ok(CancelOutcome::Cancelled {
        cancelled: CancelledThread {
            thread: closed,
            cancellation: ThreadCancellation {
                thread_id,
                reason: reason.to_string(),
                cancelled_by: actor,
                cancelled_at: now,
            },
        },
        events: stored,
    })
}

async fn get_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<Option<ThreadCancellation>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, reason, cancelled_by, cancelled_at
         FROM maidan_thread_cancellations WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| ThreadCancellation {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        reason: row.get("reason"),
        cancelled_by: MemberId(row.get::<Uuid, _>("cancelled_by")),
        cancelled_at: row.get::<DateTime<Utc>, _>("cancelled_at"),
    }))
}
