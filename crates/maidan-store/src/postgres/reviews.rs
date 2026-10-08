//! Required-reviewers store: the review requirement (`k`), the named reviewer
//! set (`n`), and reviewers' decisions. `review_status` counts the distinct
//! **qualifying** approvals the FSM close-gate (375.2) reads — decision =
//! approve, reviewer is neither owner nor assignee (SoD), and, when a named set
//! exists, is in it. See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_fsm::ThreadAction;
use maidan_types::{
    review_decision_from_waiter, Event, MemberId, ReviewDecision, ReviewStatus, ReviewSubmission,
    ReviewVerdict, ThreadId, ThreadReview, ThreadReviewRequirement, CRITICAL_REVIEW_NOTE,
    REVIEW_SKILL,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_req(row: &sqlx::postgres::PgRow) -> ThreadReviewRequirement {
    ThreadReviewRequirement {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        required_count: row.get::<i64, _>("required_count"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

fn row_to_review(row: &sqlx::postgres::PgRow) -> Result<ThreadReview, StoreError> {
    let decision_s: String = row.get("decision");
    let decision = ReviewDecision::parse(&decision_s).ok_or_else(|| {
        StoreError::InvalidInput(format!("unknown review decision: {decision_s}"))
    })?;
    Ok(ThreadReview {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        reviewer_id: MemberId(row.get::<Uuid, _>("reviewer_id")),
        decision,
        note: row.get::<Option<String>, _>("note"),
        actor_id: row.get::<Option<Uuid>, _>("actor_id").map(MemberId),
        evidence_root: row.get::<Option<String>, _>("evidence_root"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        dismissed_at: row.get::<Option<DateTime<Utc>>, _>("dismissed_at"),
    })
}

const REVIEW_COLS: &str =
    "thread_id, reviewer_id, decision, note, created_at, updated_at, actor_id, dismissed_at, evidence_root";

pub async fn set_requirement(
    pool: &PgPool,
    thread_id: ThreadId,
    required_count: i64,
) -> Result<ThreadReviewRequirement, StoreError> {
    let mut conn = pool.acquire().await?;
    set_requirement_on(&mut conn, thread_id, required_count).await
}

pub(crate) async fn set_requirement_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
    required_count: i64,
) -> Result<ThreadReviewRequirement, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_thread_review_reqs (thread_id, required_count, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW())
         ON CONFLICT (thread_id) DO UPDATE SET required_count = excluded.required_count, updated_at = NOW()
         RETURNING thread_id, required_count, created_at, updated_at",
    )
    .bind(thread_id.0)
    .bind(required_count)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_req(&row))
}

pub async fn get_requirement(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Option<ThreadReviewRequirement>, StoreError> {
    let mut conn = pool.acquire().await?;
    get_requirement_on(&mut conn, thread_id).await
}

pub(crate) async fn get_requirement_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
) -> Result<Option<ThreadReviewRequirement>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, required_count, created_at, updated_at
         FROM maidan_thread_review_reqs WHERE thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.as_ref().map(row_to_req))
}

pub async fn clear_requirement(pool: &PgPool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let mut conn = pool.acquire().await?;
    clear_requirement_on(&mut conn, thread_id).await
}

pub(crate) async fn clear_requirement_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_thread_review_reqs WHERE thread_id = $1")
        .bind(thread_id.0)
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn add_reviewer(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "INSERT INTO maidan_thread_reviewers (thread_id, member_id, created_at)
         VALUES ($1, $2, NOW()) ON CONFLICT (thread_id, member_id) DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn remove_reviewer(
    pool: &PgPool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let mut conn = pool.acquire().await?;
    remove_reviewer_on(&mut conn, thread_id, member_id).await
}

pub(crate) async fn remove_reviewer_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let done =
        sqlx::query("DELETE FROM maidan_thread_reviewers WHERE thread_id = $1 AND member_id = $2")
            .bind(thread_id.0)
            .bind(member_id.0)
            .execute(&mut *conn)
            .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn list_reviewers(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id FROM maidan_thread_reviewers WHERE thread_id = $1 ORDER BY created_at, member_id",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}

/// Record a review, and let a change request send the thread back. See the
/// SQLite twin for the rule. The thread row is locked first, so a change
/// request and a concurrent close cannot both act on `in_review`.
pub async fn submit_review(
    pool: &PgPool,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    decision: ReviewDecision,
    note: Option<&str>,
    evidence_root: Option<&str>,
) -> Result<ReviewSubmission, StoreError> {
    let mut tx = pool.begin().await?;
    let submission = submit_review_on(
        &mut tx,
        thread_id,
        reviewer_id,
        decision,
        note,
        evidence_root,
    )
    .await?;
    tx.commit().await?;
    Ok(submission)
}

/// The review, its history row and `ReviewSubmitted` (and a send-back, when
/// one applies), without committing. See the SQLite twin. Callers that have
/// more to write in the same transaction use this.
async fn submit_review_on(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    decision: ReviewDecision,
    note: Option<&str>,
    evidence_root: Option<&str>,
) -> Result<ReviewSubmission, StoreError> {
    if let Some(root) = evidence_root {
        super::review_packets::verify_root_in_tx(tx, thread_id, root).await?;
    }
    let actor_id = crate::attribution::delegate_acting_for(reviewer_id);
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_thread_reviews
             (thread_id, reviewer_id, decision, note, created_at, updated_at, actor_id, evidence_root)
         VALUES ($1, $2, $3, $4, NOW(), NOW(), $5, $6)
         ON CONFLICT (thread_id, reviewer_id) DO UPDATE SET
             decision = excluded.decision, note = excluded.note, updated_at = NOW(),
             actor_id = excluded.actor_id, dismissed_at = NULL,
             evidence_root = excluded.evidence_root
         RETURNING {REVIEW_COLS}"
    ))
    .bind(thread_id.0)
    .bind(reviewer_id.0)
    .bind(decision.as_str())
    .bind(note)
    .bind(actor_id.map(|m| m.0))
    .bind(evidence_root)
    .fetch_one(&mut **tx)
    .await?;
    let review = row_to_review(&row)?;
    // The history keeps every verdict; the row above keeps only the latest.
    sqlx::query(
        "INSERT INTO maidan_thread_review_verdicts
             (thread_id, reviewer_id, decision, note, actor_id, recorded_at, evidence_root)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(thread_id.0)
    .bind(reviewer_id.0)
    .bind(decision.as_str())
    .bind(note)
    .bind(actor_id.map(|m| m.0))
    .bind(review.updated_at)
    .bind(evidence_root)
    .execute(&mut **tx)
    .await?;
    let mut reopened = None;
    if decision == ReviewDecision::RequestChanges
        && sends_back_in_tx(tx, thread_id, reviewer_id, actor_id).await?
    {
        let result = super::thread_transitions::transition_in_tx(
            tx,
            thread_id,
            reviewer_id,
            ThreadAction::RequestChanges,
        )
        .await?;
        sqlx::query(
            "UPDATE maidan_thread_reviews SET dismissed_at = NOW()
             WHERE thread_id = $1 AND decision = 'approve' AND dismissed_at IS NULL",
        )
        .bind(thread_id.0)
        .execute(&mut **tx)
        .await?;
        reopened = Some(result);
    }
    let (workspace_id, channel_id) = super::events::thread_scope_in_tx(tx, thread_id).await?;
    let worker_id = super::thread_workers::last_worker_in_tx(tx, thread_id).await?;
    let submitted = super::events::append_in_tx(
        tx,
        &Event::ReviewSubmitted {
            occurred_at: review.updated_at,
            workspace_id,
            channel_id,
            thread_id,
            reviewer_id,
            actor_id,
            decision,
            sent_back: reopened.is_some(),
            worker_id,
        },
    )
    .await?;
    let reopened = match reopened {
        Some(result) => {
            Some(super::thread_transitions::state_changed_in_tx(tx, reviewer_id, &result).await?)
        }
        None => None,
    };
    Ok(ReviewSubmission {
        review,
        submitted,
        reopened,
    })
}

/// Whether a change request from `reviewer_id` sends the thread back (SQLite
/// twin). Locks the thread row.
async fn sends_back_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    actor_id: Option<MemberId>,
) -> Result<bool, StoreError> {
    sqlx::query("SELECT 1 FROM maidan_threads WHERE id = $1 FOR UPDATE")
        .bind(thread_id.0)
        .fetch_optional(&mut **tx)
        .await?;
    let row = sqlx::query(
        "SELECT 1 FROM maidan_threads t
         WHERE t.id = $1 AND t.state = 'in_review' AND t.tombstoned_at IS NULL
           AND (
             t.owner_id = $2
             OR (
               (t.assignee_id IS NULL OR t.assignee_id <> $2)
               AND NOT EXISTS (
                 SELECT 1 FROM maidan_thread_workers w
                 WHERE w.thread_id = t.id AND w.member_id = $2
               )
               AND (
                 NOT EXISTS (SELECT 1 FROM maidan_thread_reviewers rv WHERE rv.thread_id = t.id)
                 OR EXISTS (SELECT 1 FROM maidan_thread_reviewers rv
                            WHERE rv.thread_id = t.id AND rv.member_id = $2)
               )
             )
           )
           AND ($3::uuid IS NULL OR (
             (t.assignee_id IS NULL OR t.assignee_id <> $3)
             AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_workers wa
               WHERE wa.thread_id = t.id AND wa.member_id = $3
             )
           ))",
    )
    .bind(thread_id.0)
    .bind(reviewer_id.0)
    .bind(actor_id.map(|m| m.0))
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

pub async fn list_reviews(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadReview>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {REVIEW_COLS} FROM maidan_thread_reviews WHERE thread_id = $1 ORDER BY created_at, reviewer_id"
    ))
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_review).collect()
}

/// Count the distinct qualifying approvals + fold in the requirement. An approval
/// qualifies when: decision = approve, the reviewer is neither the thread's owner
/// nor assignee (SoD), and — when a named reviewer set exists — is in it.
/// Whether the thread's evidence differs from what its latest packet pinned.
/// `false` before any hand-off.
async fn evidence_changed(pool: &PgPool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let changed = match super::review_packets::latest_root_in_tx(&mut tx, thread_id).await? {
        Some(root) => super::review_packets::ensure_unchanged_in_tx(&mut tx, thread_id, &root)
            .await
            .is_err(),
        None => false,
    };
    tx.rollback().await?;
    Ok(changed)
}

pub async fn review_status(pool: &PgPool, thread_id: ThreadId) -> Result<ReviewStatus, StoreError> {
    let required_count: i64 = sqlx::query(
        "SELECT COALESCE(
             (SELECT required_count FROM maidan_thread_review_reqs WHERE thread_id = $1), 0)
         AS required_count",
    )
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?
    .get::<i64, _>("required_count");

    let approvals: i64 = sqlx::query(
        "SELECT COUNT(*) AS approvals FROM maidan_thread_reviews r
         JOIN maidan_threads t ON t.id = r.thread_id
         WHERE r.thread_id = $1
           AND r.decision = 'approve'
           AND r.dismissed_at IS NULL
           -- Bound to what the current review was handed, as the close gate counts.
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
           -- And never worked it. The live `assignee_id` above
           -- is cleared by a release, so on its own it let an implementer
           -- release the claim and then approve their own work.
           AND NOT EXISTS (
             SELECT 1 FROM maidan_thread_workers w
             WHERE w.thread_id = r.thread_id AND w.member_id = r.reviewer_id
           )
           -- Nor may whoever actually submitted it: a delegate that owns or
           -- worked the thread cannot approve it with a reviewer's borrowed token.
           AND (r.actor_id IS NULL OR (
             (t.owner_id IS NULL OR r.actor_id <> t.owner_id)
             AND (t.assignee_id IS NULL OR r.actor_id <> t.assignee_id)
             AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_workers wa
               WHERE wa.thread_id = r.thread_id AND wa.member_id = r.actor_id
             )
           ))
           AND (
             NOT EXISTS (SELECT 1 FROM maidan_thread_reviewers rv WHERE rv.thread_id = $1)
             OR EXISTS (SELECT 1 FROM maidan_thread_reviewers rv
                        WHERE rv.thread_id = $1 AND rv.member_id = r.reviewer_id)
           )",
    )
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?
    .get::<i64, _>("approvals");
    // As the close gate does: approvals of evidence that has since changed
    // are approvals of something else, so none of them counts.
    let approvals = if approvals > 0 && evidence_changed(pool, thread_id).await? {
        0
    } else {
        approvals
    };

    let approvals_met = required_count == 0 || approvals >= required_count;
    Ok(ReviewStatus {
        required_count,
        approvals,
        approvals_met,
    })
}

/// Persist the waiter→review map and arm the close-gate. A review-skilled
/// member plus a reviewed `example.review.result/1` with any `critical` finding
/// writes `request_changes`. If the thread has no requirement yet, this sets `k
/// = 1` so `closed` refuses until a qualifying human approve. An existing `k`
/// is left alone. The verdict and that `k` commit in one transaction: a failure
/// leaves neither, so a retry still writes the verdict instead of seeing it
/// already given and skipping the gate. See the SQLite twin.
pub async fn apply_critical_review_decision(
    pool: &PgPool,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    result: &serde_json::Value,
) -> Result<Option<ReviewSubmission>, StoreError> {
    let Some(decision) = review_decision_from_waiter(result) else {
        return Ok(None);
    };
    let skills = super::member_skills::list(pool, reviewer_id).await?;
    if !skills.iter().any(|s| s.skill == REVIEW_SKILL) {
        return Ok(None);
    }
    // The router applies this again for the same result on every replica and
    // on replay. Each verdict is an event, and a change request notifies the
    // worker, so a verdict already given on the stored result is not given
    // twice. The check, the verdict and the close-gate arm commit together.
    let mut tx = pool.begin().await?;
    if verdict_covers_result(&mut tx, thread_id, reviewer_id, decision).await? {
        return Ok(None);
    }
    let review = submit_review_on(
        &mut tx,
        thread_id,
        reviewer_id,
        decision,
        Some(CRITICAL_REVIEW_NOTE),
        None,
    )
    .await?;
    if get_requirement_on(&mut tx, thread_id).await?.is_none() {
        set_requirement_on(&mut tx, thread_id, 1).await?;
    }
    tx.commit().await?;
    Ok(Some(review))
}

/// Whether `reviewer_id`'s standing review already gives `decision` on the
/// thread's current result: it was recorded after the result was produced.
async fn verdict_covers_result(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    decision: ReviewDecision,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 FROM maidan_thread_reviews r
         JOIN maidan_thread_results tr ON tr.thread_id = r.thread_id
         WHERE r.thread_id = $1 AND r.reviewer_id = $2 AND r.decision = $3
           AND r.dismissed_at IS NULL AND r.updated_at >= tr.produced_at",
    )
    .bind(thread_id.0)
    .bind(reviewer_id.0)
    .bind(decision.as_str())
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

/// Every review verdict on a thread, oldest first.
pub async fn list_review_history(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<ReviewVerdict>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, thread_id, reviewer_id, decision, note, actor_id, recorded_at
         FROM maidan_thread_review_verdicts WHERE thread_id = $1 ORDER BY id",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            let decision: String = row.get("decision");
            Ok(ReviewVerdict {
                id: row.get("id"),
                thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
                reviewer_id: MemberId(row.get::<Uuid, _>("reviewer_id")),
                decision: ReviewDecision::parse(&decision).ok_or_else(|| {
                    StoreError::InvalidInput(format!("unknown review decision: {decision}"))
                })?,
                note: row.get("note"),
                actor_id: row.get::<Option<Uuid>, _>("actor_id").map(MemberId),
                recorded_at: row.get::<DateTime<Utc>, _>("recorded_at"),
            })
        })
        .collect()
}
