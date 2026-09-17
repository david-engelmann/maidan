//! Durable record of who has held a thread.
//!
//! Separation of duties on both governance gates tests the thread's **live**
//! `assignee_id`, and releasing a claim sets that to NULL — so the exclusion
//! became vacuous exactly when someone wanted it not to be. This table
//! remembers instead: append-only, never cleared by release or unassign.
//!
//! The event log already records assignment changes, but retention prunes it,
//! and a gate cannot depend on evidence that ages out.

use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{MemberId, ThreadId};

/// Record that `member_id` holds `thread_id`, on the caller's transaction.
///
/// Idempotent — re-claiming keeps the first timestamp, because the question the
/// gate asks is "ever", not "how often".
///
/// **On the caller's transaction on purpose.** A ledger row written outside the
/// assignment's transaction could be lost while the assignment commits, and a
/// missing row fails *open*: the gate would let the worker approve their own
/// work, which is the whole defect. Same tx or nothing.
pub async fn record_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_workers (thread_id, member_id)
         VALUES (?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Has `member_id` ever held `thread_id`? The separation-of-duties question.
pub async fn has_worked(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 AS hit FROM maidan_thread_workers
         WHERE thread_id = ? AND member_id = ? LIMIT 1",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Everyone who has ever held `thread_id`, oldest first.
pub async fn list_workers(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id FROM maidan_thread_workers
         WHERE thread_id = ? ORDER BY first_held_at ASC, member_id ASC",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| MemberId(r.get("member_id"))).collect())
}

/// [`has_worked`] on the caller's transaction.
///
/// The land gate reads this while enforcing a close, so it must see the same
/// snapshot as the rest of that transaction — a release committing between the
/// check and the close would otherwise slip through.
pub async fn has_worked_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 AS hit FROM maidan_thread_workers
         WHERE thread_id = ? AND member_id = ? LIMIT 1",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}
