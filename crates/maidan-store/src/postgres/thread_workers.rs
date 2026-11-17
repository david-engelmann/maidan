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
//! The row also carries the member's empty-release backoff from the thread
//! (`empty_releases`, `reclaim_after`; [`crate::claim_next::backed_off`]),
//! which a plain release sets or clears. That never removes the row: the
//! holding it records stays.

use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, Row};

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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
             VALUES ($1, $2) ON CONFLICT DO NOTHING",
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
             WHERE thread_id = $1)
         WHERE thread_id = $1 AND member_id = $2",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Judge `member_id`'s plain release of `thread_id`, the claim under
/// `lease_id` taken at `claimed_at`, on the release's transaction. An empty
/// hold (no usage reported under the lease, no message of the member's own
/// posted on the thread, no result stored) counts one more empty release in a
/// row and holds the thread back from the member until `now` plus
/// [`crate::claim_next::empty_release_backoff_secs`] of that count; a hold
/// that showed any of them resets both. A decline, a restart's release and a
/// lapse never come here, so they leave the backoff as it was.
pub(crate) async fn record_release_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    member_id: MemberId,
    lease_id: ClaimLeaseId,
    claimed_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    // Messages and results take their time from the database's clock, the
    // claim from the app's: the window opens early by the skew allowed for.
    // Usage is read by the claim's own token, which needs no clock.
    let since = claimed_at - Duration::seconds(crate::claim_next::HOLD_CLOCK_SLACK_SECS);
    let progressed: bool = sqlx::query(
        "SELECT EXISTS (
                    SELECT 1 FROM maidan_usage_ledger u
                    WHERE u.thread_id = $1 AND u.claim_lease_id = $3)
                OR EXISTS (
                    SELECT 1 FROM maidan_messages m
                    WHERE m.thread_id = $1 AND m.author_id = $2 AND m.posted_at >= $4)
                OR EXISTS (
                    SELECT 1 FROM maidan_thread_results r
                    WHERE r.thread_id = $1 AND r.produced_at >= $4) AS progressed",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(lease_id.0)
    .bind(since)
    .fetch_one(&mut **tx)
    .await?
    .get("progressed");
    if progressed {
        sqlx::query(
            "UPDATE maidan_thread_workers SET empty_releases = 0, reclaim_after = NULL
             WHERE thread_id = $1 AND member_id = $2",
        )
        .bind(thread_id.0)
        .bind(member_id.0)
        .execute(&mut **tx)
        .await?;
        return Ok(());
    }
    // The release holds the thread's row lock, so no other release of this
    // member's claim can read the same count.
    let previous: i32 = sqlx::query(
        "SELECT empty_releases FROM maidan_thread_workers
         WHERE thread_id = $1 AND member_id = $2",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .map_or(0, |row| row.get("empty_releases"));
    let count = previous.saturating_add(1);
    let hold = crate::claim_next::empty_release_backoff_secs(i64::from(count));
    // An upsert, since a claim older than the workers table has no row; the
    // member did hold the thread, so the row it gains is true.
    sqlx::query(
        "INSERT INTO maidan_thread_workers (thread_id, member_id, empty_releases, reclaim_after)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (thread_id, member_id) DO UPDATE
         SET empty_releases = EXCLUDED.empty_releases, reclaim_after = EXCLUDED.reclaim_after",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(count)
    .bind(now + Duration::seconds(hold))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The member who most recently took hold of `thread_id`, on the caller's
/// transaction. `None` when nobody has held it.
pub async fn last_worker_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
) -> Result<Option<MemberId>, StoreError> {
    let row = sqlx::query(
        "SELECT member_id FROM maidan_thread_workers
         WHERE thread_id = $1 AND last_held_seq IS NOT NULL
         ORDER BY last_held_seq DESC LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|r| MemberId(r.get("member_id"))))
}

/// Has `member_id` ever held `thread_id`? The separation-of-duties question.
pub async fn has_worked(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 AS hit FROM maidan_thread_workers
         WHERE thread_id = $1 AND member_id = $2 LIMIT 1",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Everyone who has ever held `thread_id`, oldest first.
pub async fn list_workers(pool: &PgPool, thread_id: ThreadId) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id FROM maidan_thread_workers
         WHERE thread_id = $1 ORDER BY first_held_at ASC, member_id ASC",
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 AS hit FROM maidan_thread_workers
         WHERE thread_id = $1 AND member_id = $2 LIMIT 1",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}
