//! Durable record of who has held a thread.
//!
//! Separation of duties on both governance gates tests the thread's **live**
//! `assignee_id`, and releasing a claim sets that to NULL — so the exclusion
//! became vacuous exactly when someone wanted it not to be. This table
//! remembers instead: append-only, never cleared by release or unassign.
//!
//! The event log already records assignment changes, but retention prunes it,
//! and a gate cannot depend on evidence that ages out.
//!
//! The row also carries the member's empty-release backoff from the thread.
//! See the Postgres twin.

use chrono::{DateTime, Duration, Utc};
use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{ClaimLeaseId, MemberId, ThreadId};

/// Record that `member_id` holds `thread_id`, on the caller's transaction.
///
/// Idempotent — re-claiming keeps the first timestamp, because the question the
/// gate asks is "ever", not "how often". It does move the member to the end
/// of the thread's `last_held_seq` order.
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
    // A delegate claiming a thread for the member it acts as has worked it too,
    // as surely as the member has. Recording only the member would let the
    // delegate go on to approve the work with a token borrowed from a reviewer.
    let actor = crate::attribution::delegate_acting_for(member_id);
    for worker in std::iter::once(member_id).chain(actor) {
        sqlx::query(
            "INSERT INTO maidan_thread_workers (thread_id, member_id)
             VALUES (?, ?) ON CONFLICT DO NOTHING",
        )
        .bind(thread_id.0)
        .bind(worker.0)
        .execute(&mut **tx)
        .await?;
    }
    // Only the member takes the next place: the delegate did the holding for
    // it, and the one to tell about a change request is the member.
    sqlx::query(
        "UPDATE maidan_thread_workers SET last_held_seq = (
             SELECT COALESCE(MAX(last_held_seq), 0) + 1 FROM maidan_thread_workers
             WHERE thread_id = ?1)
         WHERE thread_id = ?1 AND member_id = ?2",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Judge `member_id`'s plain release of `thread_id`, on the release's
/// transaction: an empty hold backs the member off the thread, any progress
/// resets the backoff. See the Postgres twin.
pub(crate) async fn record_release_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
    claimed_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    // The same window as on Postgres, so the two backends judge one hold
    // alike, though here every stamp is the app's.
    let since = claimed_at - Duration::seconds(crate::claim_next::HOLD_CLOCK_SLACK_SECS);
    let progressed: bool = sqlx::query(
        "SELECT EXISTS (
                    SELECT 1 FROM maidan_usage_ledger u
                    WHERE u.thread_id = ?1 AND u.claim_lease_id = ?3)
                OR EXISTS (
                    SELECT 1 FROM maidan_messages m
                    WHERE m.thread_id = ?1 AND m.author_id = ?2
                      AND julianday(m.posted_at) >= julianday(?4))
                OR EXISTS (
                    SELECT 1 FROM maidan_thread_results r
                    WHERE r.thread_id = ?1
                      AND julianday(r.produced_at) >= julianday(?4)) AS progressed",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .bind(since.to_rfc3339())
    .fetch_one(&mut **tx)
    .await?
    .get("progressed");
    if progressed {
        sqlx::query(
            "UPDATE maidan_thread_workers SET empty_releases = 0, reclaim_after = NULL
             WHERE thread_id = ? AND member_id = ?",
        )
        .bind(thread_id.0)
        .bind(member_id.0)
        .execute(&mut **tx)
        .await?;
        return Ok(());
    }
    // The release's transaction holds SQLite's write lock, so no other
    // release can read the same count.
    let previous: i64 = sqlx::query(
        "SELECT empty_releases FROM maidan_thread_workers
         WHERE thread_id = ? AND member_id = ?",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .map_or(0, |row| row.get("empty_releases"));
    // Kept within Postgres's INTEGER, so the two backends count alike.
    let count = previous.saturating_add(1).min(i64::from(i32::MAX));
    let hold = crate::claim_next::empty_release_backoff_secs(count);
    sqlx::query(
        "INSERT INTO maidan_thread_workers (thread_id, member_id, empty_releases, reclaim_after)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (thread_id, member_id) DO UPDATE
         SET empty_releases = excluded.empty_releases, reclaim_after = excluded.reclaim_after",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(count)
    .bind((now + Duration::seconds(hold)).to_rfc3339())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The member who most recently took hold of `thread_id`, on the caller's
/// transaction. `None` when nobody has held it.
pub async fn last_worker_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<Option<MemberId>, StoreError> {
    let row = sqlx::query(
        "SELECT member_id FROM maidan_thread_workers
         WHERE thread_id = ?1 AND last_held_seq IS NOT NULL
         ORDER BY last_held_seq DESC LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|r| MemberId(r.get("member_id"))))
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
