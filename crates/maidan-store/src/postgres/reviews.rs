//! Required-reviewers store: the review requirement (`k`), the named reviewer
//! set (`n`), and reviewers' decisions. `review_status` counts the distinct
//! **qualifying** approvals the FSM close-gate (375.2) reads — decision =
//! approve, reviewer is neither owner nor assignee (SoD), and, when a named set
//! exists, is in it. See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_types::{
    review_decision_from_waiter, MemberId, ReviewDecision, ReviewStatus, ThreadId, ThreadReview,
    ThreadReviewRequirement, CRITICAL_REVIEW_NOTE, REVIEW_SKILL,
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
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}

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
        "SELECT member_id FROM maidan_thread_reviewers WHERE thread_id = $1 ORDER BY created_at",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}

pub async fn submit_review(
    pool: &PgPool,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    decision: ReviewDecision,
    note: Option<&str>,
) -> Result<ThreadReview, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_thread_reviews
             (thread_id, reviewer_id, decision, note, created_at, updated_at, actor_id)
         VALUES ($1, $2, $3, $4, NOW(), NOW(), $5)
         ON CONFLICT (thread_id, reviewer_id) DO UPDATE SET
             decision = excluded.decision, note = excluded.note, updated_at = NOW(),
             actor_id = excluded.actor_id
         RETURNING thread_id, reviewer_id, decision, note, created_at, updated_at, actor_id",
    )
    .bind(thread_id.0)
    .bind(reviewer_id.0)
    .bind(decision.as_str())
    .bind(note)
    .bind(crate::attribution::delegate_acting_for(reviewer_id).map(|m| m.0))
    .fetch_one(pool)
    .await?;
    row_to_review(&row)
}

pub async fn list_reviews(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadReview>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, reviewer_id, decision, note, created_at, updated_at, actor_id
         FROM maidan_thread_reviews WHERE thread_id = $1 ORDER BY created_at",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_review).collect()
}

/// Count the distinct qualifying approvals + fold in the requirement. An approval
/// qualifies when: decision = approve, the reviewer is neither the thread's owner
/// nor assignee (SoD), and — when a named reviewer set exists — is in it.
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
/// is left alone.
pub async fn apply_critical_review_decision(
    pool: &PgPool,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    result: &serde_json::Value,
) -> Result<Option<ThreadReview>, StoreError> {
    let Some(decision) = review_decision_from_waiter(result) else {
        return Ok(None);
    };
    let skills = super::member_skills::list(pool, reviewer_id).await?;
    if !skills.iter().any(|s| s.skill == REVIEW_SKILL) {
        return Ok(None);
    }
    let review = submit_review(
        pool,
        thread_id,
        reviewer_id,
        decision,
        Some(CRITICAL_REVIEW_NOTE),
    )
    .await?;
    if get_requirement(pool, thread_id).await?.is_none() {
        set_requirement(pool, thread_id, 1).await?;
    }
    Ok(Some(review))
}
