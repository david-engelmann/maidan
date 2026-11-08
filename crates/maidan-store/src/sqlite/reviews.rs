//! Required-reviewers store. `review_status` counts the distinct qualifying
//! approvals the FSM close-gate reads — decision = approve, reviewer is neither
//! owner nor assignee (SoD), and, when a named set exists, is in it.

use chrono::{DateTime, Utc};
use maidan_fsm::ThreadAction;
use maidan_types::{
    review_decision_from_waiter, review_round_cap_prompt, ApprovalGate, Event, MemberId,
    ReviewDecision, ReviewStatus, ReviewSubmission, ReviewVerdict, ThreadId, ThreadReview,
    ThreadReviewRequirement, WorkspaceId, CRITICAL_REVIEW_NOTE, DEFAULT_MAX_REVIEW_ROUNDS,
    REVIEW_SKILL,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_req(row: &sqlx::sqlite::SqliteRow) -> ThreadReviewRequirement {
    ThreadReviewRequirement {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        required_count: row.get::<i64, _>("required_count"),
        max_rounds: row.get::<i64, _>("max_rounds"),
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
        actor_id: row.get::<Option<Uuid>, _>("actor_id").map(MemberId),
        evidence_root: row.get::<Option<String>, _>("evidence_root"),
        head_sha: row.get::<Option<String>, _>("head_sha"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        dismissed_at: row.get::<Option<DateTime<Utc>>, _>("dismissed_at"),
    })
}

const REVIEW_COLS: &str = "thread_id, reviewer_id, decision, note, created_at, updated_at, actor_id, dismissed_at, evidence_root, head_sha";

#[cfg(feature = "test-support")]
pub async fn set_requirement(
    pool: &SqlitePool,
    thread_id: ThreadId,
    required_count: i64,
) -> Result<ThreadReviewRequirement, StoreError> {
    let mut conn = pool.acquire().await?;
    set_requirement_on(&mut conn, thread_id, required_count, None).await
}

#[cfg(feature = "test-support")]
pub async fn set_rounds(
    pool: &SqlitePool,
    thread_id: ThreadId,
    required_count: i64,
    max_rounds: Option<i64>,
) -> Result<ThreadReviewRequirement, StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    // Same critical section as the audited setter: read the thread, then write.
    sqlx::query("SELECT 1 FROM maidan_threads WHERE id = ?")
        .bind(thread_id.0)
        .fetch_optional(&mut *tx)
        .await?;
    let requirement = set_requirement_on(&mut tx, thread_id, required_count, max_rounds).await?;
    tx.commit().await?;
    Ok(requirement)
}

/// An explicit claim or assignment is refused while a review-round cap is
/// pending, or after one was declined or cancelled and parked the thread.
/// `claim_next` already skips both; this is the same rule for a named claim.
pub(crate) async fn refuse_round_cap_claim(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    let pending: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM maidan_approval_gates
         WHERE thread_id = ? AND kind = 'review_round_cap' AND state = 'pending'
         LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    let parked: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM maidan_thread_unclaimable WHERE thread_id = ? AND reason = ? LIMIT 1",
    )
    .bind(thread_id.0)
    .bind(maidan_types::REVIEW_ROUND_HELD_REASON)
    .fetch_optional(&mut *conn)
    .await?;
    if pending.is_some() || parked.is_some() {
        return Err(StoreError::Conflict(
            maidan_types::REVIEW_ROUND_CLAIM_REFUSAL.into(),
        ));
    }
    Ok(())
}

pub(crate) async fn set_requirement_on(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
    required_count: i64,
    max_rounds: Option<i64>,
) -> Result<ThreadReviewRequirement, StoreError> {
    if required_count < 0 || max_rounds.is_some_and(|n| n < 0) {
        return Err(StoreError::InvalidInput(
            "required_count and max_rounds must be >= 0".into(),
        ));
    }
    let now = Utc::now().to_rfc3339();
    // A missing max_rounds keeps the row's cap, or the default when inserting.
    let row = sqlx::query(
        "INSERT INTO maidan_thread_review_reqs
             (thread_id, required_count, max_rounds, created_at, updated_at)
         VALUES (?, ?, COALESCE(?, 3), ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             required_count = excluded.required_count,
             max_rounds = CASE WHEN ? IS NULL THEN maidan_thread_review_reqs.max_rounds
                               ELSE excluded.max_rounds END,
             updated_at = excluded.updated_at
         RETURNING thread_id, required_count, max_rounds, created_at, updated_at",
    )
    .bind(thread_id.0)
    .bind(required_count)
    .bind(max_rounds)
    .bind(&now)
    .bind(&now)
    .bind(max_rounds)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_req(&row))
}

pub async fn get_requirement(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadReviewRequirement>, StoreError> {
    let mut conn = pool.acquire().await?;
    get_requirement_on(&mut conn, thread_id).await
}

pub(crate) async fn get_requirement_on(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
) -> Result<Option<ThreadReviewRequirement>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, required_count, max_rounds, created_at, updated_at
         FROM maidan_thread_review_reqs WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.as_ref().map(row_to_req))
}

#[cfg(feature = "test-support")]
pub async fn clear_requirement(pool: &SqlitePool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let mut conn = pool.acquire().await?;
    clear_requirement_on(&mut conn, thread_id).await
}

pub(crate) async fn clear_requirement_on(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_thread_review_reqs WHERE thread_id = ?")
        .bind(thread_id.0)
        .execute(&mut *conn)
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

#[cfg(feature = "test-support")]
pub async fn remove_reviewer(
    pool: &SqlitePool,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let mut conn = pool.acquire().await?;
    remove_reviewer_on(&mut conn, thread_id, member_id).await
}

pub(crate) async fn remove_reviewer_on(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let done =
        sqlx::query("DELETE FROM maidan_thread_reviewers WHERE thread_id = ? AND member_id = ?")
            .bind(thread_id.0)
            .bind(member_id.0)
            .execute(&mut *conn)
            .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn list_reviewers(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id FROM maidan_thread_reviewers WHERE thread_id = ? ORDER BY created_at, member_id",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}

/// Record a review, and let a change request send the thread back.
///
/// A `request_changes` review on a thread under review, from its owner or from
/// a reviewer whose approval would count, returns the thread to `open` in the
/// same transaction: the thread is claimable again, the approvals given to the
/// version it replaces are dismissed, and the `ThreadStateChanged` event is
/// returned for the caller to publish. From anyone else, or on a thread not
/// under review, the review is only recorded, the way an implementer's
/// approval is recorded and not counted.
pub async fn submit_review(
    pool: &SqlitePool,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    decision: ReviewDecision,
    note: Option<&str>,
    evidence_root: Option<&str>,
) -> Result<ReviewSubmission, StoreError> {
    submit_review_at_head(
        pool,
        thread_id,
        reviewer_id,
        decision,
        note,
        evidence_root,
        None,
    )
    .await
}

/// [`submit_review`], naming the head the reviewer read (#1574). A named
/// head that is not the change's tracked head is a conflict: the change
/// moved since the reviewer read it. The verdict stores the named head.
#[allow(clippy::too_many_arguments)]
pub async fn submit_review_at_head(
    pool: &SqlitePool,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    decision: ReviewDecision,
    note: Option<&str>,
    evidence_root: Option<&str>,
    head_sha: Option<&str>,
) -> Result<ReviewSubmission, StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    let submission = submit_review_on(
        &mut tx,
        thread_id,
        reviewer_id,
        decision,
        note,
        evidence_root,
        head_sha,
    )
    .await?;
    tx.commit().await?;
    Ok(submission)
}

/// The review, its history row and `ReviewSubmitted` (and a send-back, when
/// one applies), without committing. Callers that have more to write in the
/// same transaction use this.
async fn submit_review_on(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    decision: ReviewDecision,
    note: Option<&str>,
    evidence_root: Option<&str>,
    named_head: Option<&str>,
) -> Result<ReviewSubmission, StoreError> {
    if let Some(root) = evidence_root {
        super::review_packets::verify_root_in_tx(tx, thread_id, root).await?;
    }
    let actor_id = crate::attribution::delegate_acting_for(reviewer_id);
    // The verdict names the head of the change it was given against.
    let tracked: Option<String> = sqlx::query_scalar(&format!(
        "SELECT {}",
        crate::review_head::current_head("?1")
    ))
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    let head_sha = match named_head {
        None => tracked,
        Some(named) => {
            // A head nothing tracks can't be checked, so it isn't stored: a
            // verdict would otherwise name a head nobody verified (#1744).
            let Some(tracked) = tracked.as_deref() else {
                return Err(StoreError::Conflict(format!(
                    "unverified head: you named {named} but this change has no tracked pull request head to check it against; omit head_sha or register the pull request first"
                )));
            };
            if !tracked.eq_ignore_ascii_case(named) {
                return Err(StoreError::Conflict(format!(
                    "stale head: you reviewed {named} but the change's head is now {tracked}; re-read the change and review again"
                )));
            }
            Some(named.to_ascii_lowercase())
        }
    };
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_thread_reviews
             (thread_id, reviewer_id, decision, note, created_at, updated_at, actor_id, evidence_root, head_sha)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (thread_id, reviewer_id) DO UPDATE SET
             decision = excluded.decision, note = excluded.note, updated_at = excluded.updated_at,
             actor_id = excluded.actor_id, dismissed_at = NULL,
             evidence_root = excluded.evidence_root, head_sha = excluded.head_sha
         RETURNING {REVIEW_COLS}"
    ))
    .bind(thread_id.0)
    .bind(reviewer_id.0)
    .bind(decision.as_str())
    .bind(note)
    .bind(&now)
    .bind(&now)
    .bind(actor_id.map(|m| m.0))
    .bind(evidence_root)
    .bind(head_sha.as_deref())
    .fetch_one(&mut **tx)
    .await?;
    let review = row_to_review(&row)?;
    // The history keeps every verdict; the row above keeps only the latest.
    sqlx::query(
        "INSERT INTO maidan_thread_review_verdicts
             (thread_id, reviewer_id, decision, note, actor_id, recorded_at, evidence_root, head_sha)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(thread_id.0)
    .bind(reviewer_id.0)
    .bind(decision.as_str())
    .bind(note)
    .bind(actor_id.map(|m| m.0))
    .bind(&now)
    .bind(evidence_root)
    .bind(head_sha.as_deref())
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
            "UPDATE maidan_thread_reviews SET dismissed_at = ?
             WHERE thread_id = ? AND decision = 'approve' AND dismissed_at IS NULL",
        )
        .bind(&now)
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
    let (escalated, approval_gate) = if reopened.is_some() {
        escalate_round_cap(tx, workspace_id, channel_id, thread_id, reviewer_id, note).await?
    } else {
        (None, None)
    };
    Ok(ReviewSubmission {
        review,
        submitted,
        reopened,
        escalated,
        approval_gate,
    })
}

/// Send-backs since the latest accepted review-round cap, the cap in force,
/// and whether a review-round gate is already pending. Times are compared as
/// `DateTime<Utc>`, not as stored text.
async fn round_standing(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
) -> Result<(i64, i64, bool), StoreError> {
    let max_rounds: i64 =
        sqlx::query_scalar("SELECT max_rounds FROM maidan_thread_review_reqs WHERE thread_id = ?")
            .bind(thread_id.0)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(DEFAULT_MAX_REVIEW_ROUNDS);
    let cutoff: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT resolved_at FROM maidan_approval_gates
         WHERE thread_id = ? AND kind = 'review_round_cap' AND state = 'accepted'
           AND resolved_at IS NOT NULL
         ORDER BY resolved_at DESC
         LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    let times: Vec<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT occurred_at FROM maidan_thread_transitions
         WHERE thread_id = ? AND from_state = 'in_review' AND to_state = 'open'",
    )
    .bind(thread_id.0)
    .fetch_all(&mut *conn)
    .await?;
    let send_backs = times
        .into_iter()
        .filter(|at| cutoff.is_none_or(|cut| *at > cut))
        .count() as i64;
    let pending: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM maidan_approval_gates
         WHERE thread_id = ? AND kind = 'review_round_cap' AND state = 'pending'
         LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    Ok((max_rounds, send_backs, pending.is_some()))
}

pub(crate) async fn escalate_round_cap(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    workspace_id: WorkspaceId,
    channel_id: maidan_types::ChannelId,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    note: Option<&str>,
) -> Result<(Option<maidan_types::StoredEvent>, Option<ApprovalGate>), StoreError> {
    let (max_rounds, send_backs, pending) = round_standing(tx, thread_id).await?;
    if max_rounds == 0 || pending || send_backs < max_rounds {
        return Ok((None, None));
    }
    // The worker may still hold the lease that brought the thread to review.
    // Drop it in this transaction so a renew or a result write cannot land
    // while the person has not decided.
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE maidan_threads
         SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL,
             claimed_at = NULL, work_started_at = NULL, updated_at = ?
         WHERE id = ?",
    )
    .bind(&now)
    .bind(thread_id.0)
    .execute(&mut **tx)
    .await?;
    let gate = super::approval_gates::open_review_round_cap(
        tx,
        workspace_id,
        thread_id,
        reviewer_id,
        &review_round_cap_prompt(send_backs, note),
    )
    .await?;
    let stored = super::events::append_in_tx(
        tx,
        &Event::ApprovalRequested {
            occurred_at: gate.created_at,
            workspace_id,
            channel_id: Some(channel_id),
            thread_id: Some(thread_id),
            gate_id: gate.id,
            requested_by: reviewer_id,
        },
    )
    .await?;
    Ok((Some(stored), Some(gate)))
}

/// Whether a change request from `reviewer_id` sends the thread back: it is
/// under review, and the reviewer owns it or is a reviewer whose approval
/// would count. Whoever submitted it for them must not have worked it either.
async fn sends_back_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    actor_id: Option<MemberId>,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 FROM maidan_threads t
         WHERE t.id = ?1 AND t.state = 'in_review' AND t.tombstoned_at IS NULL
           AND (
             t.owner_id = ?2
             OR (
               (t.assignee_id IS NULL OR t.assignee_id <> ?2)
               AND NOT EXISTS (
                 SELECT 1 FROM maidan_thread_workers w
                 WHERE w.thread_id = t.id AND w.member_id = ?2
               )
               AND (
                 NOT EXISTS (SELECT 1 FROM maidan_thread_reviewers rv WHERE rv.thread_id = t.id)
                 OR EXISTS (SELECT 1 FROM maidan_thread_reviewers rv
                            WHERE rv.thread_id = t.id AND rv.member_id = ?2)
               )
             )
           )
           AND (?3 IS NULL OR (
             (t.assignee_id IS NULL OR t.assignee_id <> ?3)
             AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_workers wa
               WHERE wa.thread_id = t.id AND wa.member_id = ?3
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
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadReview>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {REVIEW_COLS} FROM maidan_thread_reviews WHERE thread_id = ? ORDER BY created_at, reviewer_id"
    ))
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_review).collect()
}

/// Whether the thread's evidence differs from what its latest packet pinned.
/// `false` before any hand-off.
async fn evidence_changed(pool: &SqlitePool, thread_id: ThreadId) -> Result<bool, StoreError> {
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
        "SELECT COUNT(DISTINCT COALESCE(r.actor_id, r.reviewer_id)) AS approvals FROM maidan_thread_reviews r
         JOIN maidan_threads t ON t.id = r.thread_id
         WHERE r.thread_id = ?
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
    // As the close gate does: approvals of evidence that has since changed
    // are approvals of something else, so none of them counts.
    let approvals = if approvals > 0 && evidence_changed(pool, thread_id).await? {
        0
    } else {
        approvals
    };

    let approvals_met = required_count == 0 || approvals >= required_count;
    let mut conn = pool.acquire().await?;
    let (max_rounds, send_backs, _) = round_standing(&mut conn, thread_id).await?;
    Ok(ReviewStatus {
        required_count,
        approvals,
        approvals_met,
        send_backs,
        max_rounds,
    })
}

/// Persist the waiter→review map and arm the close-gate. A review-skilled
/// member plus a reviewed `example.review.result/1` with any `critical` finding
/// writes `request_changes`. If the thread has no requirement yet, this sets `k
/// = 1` so `closed` refuses until a qualifying human approve. An existing `k`
/// is left alone. The verdict and that `k` commit in one transaction: a failure
/// leaves neither, so a retry still writes the verdict instead of seeing it
/// already given and skipping the gate.
pub async fn apply_critical_review_decision(
    pool: &SqlitePool,
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
    let mut tx = super::tx::begin_write(pool).await?;
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
        None,
    )
    .await?;
    if get_requirement_on(&mut tx, thread_id).await?.is_none() {
        set_requirement_on(&mut tx, thread_id, 1, None).await?;
    }
    tx.commit().await?;
    Ok(Some(review))
}

/// Whether `reviewer_id`'s standing review already gives `decision` on the
/// thread's current result: it was recorded after the result was produced.
async fn verdict_covers_result(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    reviewer_id: MemberId,
    decision: ReviewDecision,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 FROM maidan_thread_reviews r
         JOIN maidan_thread_results tr ON tr.thread_id = r.thread_id
         WHERE r.thread_id = ? AND r.reviewer_id = ? AND r.decision = ?
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
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<ReviewVerdict>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, thread_id, reviewer_id, decision, note, actor_id, recorded_at, head_sha
         FROM maidan_thread_review_verdicts WHERE thread_id = ? ORDER BY id",
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
                head_sha: row.get("head_sha"),
                recorded_at: row.get::<DateTime<Utc>, _>("recorded_at"),
            })
        })
        .collect()
}
