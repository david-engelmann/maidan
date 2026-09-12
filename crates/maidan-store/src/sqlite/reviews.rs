//! Required-reviewers store (Cluster 375, Wave 2 #22, SQLite twin of pg 0079).
//! `review_status` counts the distinct qualifying approvals the FSM close-gate
//! reads — decision = approve, reviewer is neither owner nor assignee (SoD), and,
//! when a named set exists, is in it.

use chrono::{DateTime, Utc};
use maidan_types::{
    MemberId, ReviewDecision, ReviewStatus, ThreadId, ThreadReview, ThreadReviewRequirement,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_req(row: &sqlx::sqlite::SqliteRow) -> ThreadReviewRequirement {
    ThreadReviewRequirement {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        required_count: row.get::<i64, _>("required_count"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

fn row_to_review(row: &sqlx::sqlite::SqliteRow) -> Result<ThreadReview, StoreError> {
    let decision_s: String = row.get("decision");
    let decision = ReviewDecision::parse(&decision_s).ok_or_else(|| {
        StoreError::InvalidInput(format!("unknown review decision: {decision_s}"))
    })?;
    Ok(ThreadReview {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        reviewer_id: MemberId(row.get::<Uuid, _>("reviewer_id")),
        decision,
        note: row.get::<Option<String>, _>("note"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}

pub async fn set_requirement(
    pool: &SqlitePool,
    thread_id: ThreadId,
    required_count: i64,
) -> Result<ThreadReviewRequirement, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_thread_review_reqs (thread_id, required_count, created_at, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET required_count = excluded.required_count, updated_at = excluded.updated_at
         RETURNING thread_id, required_count, created_at, updated_at",
    )
    .bind(thread_id.0)
    .bind(required_count)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_req(&row))
}

pub async fn get_requirement(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadReviewRequirement>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, required_count, created_at, updated_at
         FROM maidan_thread_review_reqs WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_req))
}

pub async fn clear_requirement(pool: &SqlitePool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_thread_review_reqs WHERE thread_id = ?")
        .bind(thread_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn add_reviewer(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let now = Utc::now().to_rfc3339();
    let done = sqlx::query(
        "INSERT INTO maidan_thread_reviewers (thread_id, member_id, created_at)
         VALUES (?, ?, ?) ON CONFLICT (thread_id, member_id) DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(member_id.0)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn remove_reviewer(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let done =
        sqlx::query("DELETE FROM maidan_thread_reviewers WHERE thread_id = ? AND member_id = ?")
            .bind(thread_id.0)
            .bind(member_id.0)
            .execute(pool)
            .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn list_reviewers(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id FROM maidan_thread_reviewers WHERE thread_id = ? ORDER BY created_at",
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
    pool: &SqlitePool,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    decision: ReviewDecision,
    note: Option<&str>,
) -> Result<ThreadReview, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_thread_reviews (thread_id, reviewer_id, decision, note, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (thread_id, reviewer_id) DO UPDATE SET
             decision = excluded.decision, note = excluded.note, updated_at = excluded.updated_at
         RETURNING thread_id, reviewer_id, decision, note, created_at, updated_at",
    )
    .bind(thread_id.0)
    .bind(reviewer_id.0)
    .bind(decision.as_str())
    .bind(note)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    row_to_review(&row)
}

pub async fn list_reviews(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadReview>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, reviewer_id, decision, note, created_at, updated_at
         FROM maidan_thread_reviews WHERE thread_id = ? ORDER BY created_at",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_review).collect()
}

pub async fn review_status(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<ReviewStatus, StoreError> {
    let required_count: i64 = sqlx::query(
        "SELECT COALESCE(
             (SELECT required_count FROM maidan_thread_review_reqs WHERE thread_id = ?), 0)
         AS required_count",
    )
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?
    .get::<i64, _>("required_count");

    // Positional binds: thread_id is used three times (the row filter + the two
    // named-set EXISTS subqueries).
    let approvals: i64 = sqlx::query(
        "SELECT COUNT(*) AS approvals FROM maidan_thread_reviews r
         JOIN maidan_threads t ON t.id = r.thread_id
         WHERE r.thread_id = ?
           AND r.decision = 'approve'
           AND (t.owner_id IS NULL OR r.reviewer_id <> t.owner_id)
           AND (t.assignee_id IS NULL OR r.reviewer_id <> t.assignee_id)
           AND (
             NOT EXISTS (SELECT 1 FROM maidan_thread_reviewers rv WHERE rv.thread_id = ?)
             OR EXISTS (SELECT 1 FROM maidan_thread_reviewers rv
                        WHERE rv.thread_id = ? AND rv.member_id = r.reviewer_id)
           )",
    )
    .bind(thread_id.0)
    .bind(thread_id.0)
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
