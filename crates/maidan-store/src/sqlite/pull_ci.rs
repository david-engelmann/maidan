//! CI on tracked pull requests on SQLite; see the Postgres twin.

use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, CiStatus, Event, MemberId, MergeQueue, MergeQueueState, NewPullCi, PostedNotice,
    PreviousRead, PullCi, StoredEvent, ThreadId, WorkspaceId,
};
use sqlx::types::Json;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::change_metrics::insert_notice_in_tx;
use crate::error::StoreError;

/// The thread `repo`#`pull_number` is tracked on in `workspace_id`; `None`
/// when this workspace does not track it, whoever else does.
pub async fn tracked_thread(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
) -> Result<Option<ThreadId>, StoreError> {
    let repo = repo.to_ascii_lowercase();
    let thread: Option<Uuid> = sqlx::query_scalar(
        "SELECT thread_id FROM maidan_change_pulls
         WHERE workspace_id = ?1 AND repo = ?2 AND pull_number = ?3",
    )
    .bind(workspace_id.0)
    .bind(&repo)
    .bind(pull_number)
    .fetch_optional(pool)
    .await?;
    Ok(thread.map(ThreadId))
}

/// Whether `head_sha` was the head the last read of `repo`#`pull_number`
/// in `workspace_id` recorded, with commits its base lacked.
pub async fn head_was_ahead(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
    head_sha: &str,
) -> Result<bool, StoreError> {
    let ahead: Option<bool> = sqlx::query_scalar(
        "SELECT head_ahead FROM maidan_pull_ci
         WHERE workspace_id = ?1 AND repo = ?2 AND pull_number = ?3 AND head_sha = ?4",
    )
    .bind(workspace_id.0)
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(head_sha.to_ascii_lowercase())
    .fetch_optional(pool)
    .await?;
    Ok(ahead.unwrap_or(false))
}

/// The last read of a tracked pull request's checks in `workspace_id`.
pub async fn get(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
) -> Result<Option<PullCi>, StoreError> {
    let row = sqlx::query(
        "SELECT c.repo, c.pull_number, c.head_sha, c.status, c.failing_jobs, c.log_tail, c.updated_at,
                c.merge_state, c.unresolved_threads, c.unresolved_thread_links, c.missing_checks,
                c.gate_reasons, c.merge_queue, c.merge_queue_reason,
                EXISTS (SELECT 1 FROM maidan_change_pulls p
                        WHERE p.repo = c.repo AND p.pull_number = c.pull_number
                          AND p.workspace_id = c.workspace_id
                          AND p.merged_at IS NOT NULL) AS landed
         FROM maidan_pull_ci c
         WHERE c.workspace_id = ?1 AND c.repo = ?2 AND c.pull_number = ?3",
    )
    .bind(workspace_id.0)
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(ci_from).transpose()
}

/// The last reads of the checks on every pull request tracked on a thread.
pub async fn list_for_thread(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<PullCi>, StoreError> {
    let rows = sqlx::query(
        "SELECT c.repo, c.pull_number, c.head_sha, c.status, c.failing_jobs, c.log_tail, c.updated_at,
                c.merge_state, c.unresolved_threads, c.unresolved_thread_links, c.missing_checks,
                c.gate_reasons, c.merge_queue, c.merge_queue_reason,
                p.merged_at IS NOT NULL AS landed
         FROM maidan_pull_ci c
         JOIN maidan_change_pulls p
           ON p.repo = c.repo AND p.pull_number = c.pull_number AND p.workspace_id = c.workspace_id
         WHERE p.thread_id = ?1
         ORDER BY c.repo, c.pull_number",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(ci_from).collect()
}

/// Record a read of a tracked pull request's checks and the notices it owes
/// its thread, in one transaction. Each notice's key says it once, so a
/// second replica's read of the same head records nothing new. Nothing when
/// the pull request is not tracked in `new.workspace_id`.
pub async fn record(pool: &SqlitePool, new: NewPullCi) -> Result<Vec<PostedNotice>, StoreError> {
    Ok(record_with_events(pool, new).await?.0)
}

/// [`record`], also returning the `approval_dismissed` events a moved head
/// appended (#1572), for the caller to publish live.
pub async fn record_with_events(
    pool: &SqlitePool,
    new: NewPullCi,
) -> Result<(Vec<PostedNotice>, Vec<StoredEvent>), StoreError> {
    let repo = new.repo.to_ascii_lowercase();
    let mut tx = pool.begin().await?;
    let thread: Option<Uuid> = sqlx::query_scalar(
        "SELECT thread_id FROM maidan_change_pulls
         WHERE workspace_id = ?1 AND repo = ?2 AND pull_number = ?3",
    )
    .bind(new.workspace_id.0)
    .bind(&repo)
    .bind(new.pull_number)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(thread) = thread else {
        return Ok((Vec::new(), Vec::new()));
    };
    let previous = sqlx::query(
        "SELECT gate_reasons, gate_said, merge_queue, merge_queue_reason FROM maidan_pull_ci
         WHERE workspace_id = ?1 AND repo = ?2 AND pull_number = ?3",
    )
    .bind(new.workspace_id.0)
    .bind(&repo)
    .bind(new.pull_number)
    .fetch_optional(&mut *tx)
    .await?
    .map(|row| PreviousRead {
        gate: row
            .get::<Option<Json<Vec<String>>>, _>("gate_reasons")
            .map(|reasons| reasons.0),
        merge_queue: queue_from(&row, false),
        gate_said: row
            .get::<Option<Json<Vec<String>>>, _>("gate_said")
            .map(|reasons| reasons.0),
    })
    .unwrap_or_default();
    let said = maidan_types::gate_to_say(new.gate_reasons.as_deref(), &previous);
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_pull_ci
             (repo, pull_number, workspace_id, head_sha, status, failing_jobs, log_tail, updated_at,
              merge_state, unresolved_threads, unresolved_thread_links, missing_checks, gate_reasons,
              merge_queue, merge_queue_reason, head_ahead, gate_said)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT (workspace_id, repo, pull_number) DO UPDATE SET
             head_sha = excluded.head_sha, status = excluded.status,
             failing_jobs = excluded.failing_jobs, log_tail = excluded.log_tail,
             updated_at = excluded.updated_at, merge_state = excluded.merge_state,
             unresolved_threads = excluded.unresolved_threads,
             unresolved_thread_links = excluded.unresolved_thread_links,
             missing_checks = excluded.missing_checks,
             gate_reasons = COALESCE(excluded.gate_reasons, maidan_pull_ci.gate_reasons),
             merge_queue = CASE WHEN excluded.gate_reasons IS NULL
                 THEN maidan_pull_ci.merge_queue ELSE excluded.merge_queue END,
             merge_queue_reason = CASE WHEN excluded.gate_reasons IS NULL
                 THEN maidan_pull_ci.merge_queue_reason ELSE excluded.merge_queue_reason END,
             head_ahead = CASE WHEN maidan_pull_ci.head_sha = excluded.head_sha
                 THEN (maidan_pull_ci.head_ahead OR excluded.head_ahead)
                 ELSE excluded.head_ahead END,
             gate_said = COALESCE(excluded.gate_said, maidan_pull_ci.gate_said)",
    )
    .bind(&repo)
    .bind(new.pull_number)
    .bind(new.workspace_id.0)
    .bind(new.head_sha.to_ascii_lowercase())
    .bind(new.status.as_str())
    .bind(Json(&new.failing_jobs))
    .bind(&new.log_tail)
    .bind(now.to_rfc3339())
    .bind(&new.merge_state)
    .bind(new.unresolved_threads)
    .bind(Json(&new.unresolved_thread_links))
    .bind(Json(&new.missing_checks))
    .bind(new.gate_reasons.as_ref().map(Json))
    .bind(new.merge_queue.as_ref().map(|queue| queue.state.as_str()))
    .bind(new.merge_queue.as_ref().and_then(|queue| queue.reason.clone()))
    .bind(new.head_ahead)
    .bind(said.as_ref().map(Json))
    .execute(&mut *tx)
    .await?;
    // A push that moved the head dismisses approvals given to an earlier one.
    let head = new.head_sha.to_ascii_lowercase();
    let dismissed_rows = sqlx::query(&crate::review_head::dismiss_stale("?1", "?2", "?3"))
        .bind(thread)
        .bind(&head)
        .bind(now.to_rfc3339())
        .fetch_all(&mut *tx)
        .await?;
    let mut dismissed = Vec::with_capacity(dismissed_rows.len());
    for row in &dismissed_rows {
        let thread_id: Uuid = row.get("thread_id");
        let channel_id: Uuid =
            sqlx::query_scalar("SELECT channel_id FROM maidan_threads WHERE id = ?1")
                .bind(thread_id)
                .fetch_one(&mut *tx)
                .await?;
        let event = Event::ApprovalDismissed {
            occurred_at: now,
            workspace_id: new.workspace_id,
            channel_id: ChannelId(channel_id),
            thread_id: ThreadId(thread_id),
            reviewer_id: MemberId(row.get("reviewer_id")),
            approved_head: row.get("head_sha"),
            head_sha: head.clone(),
        };
        dismissed.push(super::events::append_in_tx(&mut tx, &event).await?);
    }
    let mut posted = Vec::new();
    let change = Uuid::now_v7().to_string();
    for (notice, key) in maidan_types::ci_notices(&new, &previous, &change, now) {
        if let Some(p) = insert_notice_in_tx(&mut tx, ThreadId(thread), notice, &key).await? {
            posted.push(p);
        }
    }
    tx.commit().await?;
    Ok((posted, dismissed))
}

fn ci_from(row: &sqlx::sqlite::SqliteRow) -> Result<PullCi, StoreError> {
    let status: String = row.get("status");
    Ok(PullCi {
        repo: row.get("repo"),
        pull_number: row.get("pull_number"),
        head_sha: row.get("head_sha"),
        status: CiStatus::parse(&status)
            .ok_or_else(|| StoreError::InvalidInput(format!("unknown CI status `{status}`")))?,
        failing_jobs: row.get::<Json<Vec<String>>, _>("failing_jobs").0,
        log_tail: row.get("log_tail"),
        merge_state: row.get("merge_state"),
        unresolved_threads: row.get("unresolved_threads"),
        unresolved_thread_links: row
            .get::<Json<Vec<maidan_types::ReviewThreadLink>>, _>("unresolved_thread_links")
            .0,
        missing_checks: row.get::<Json<Vec<String>>, _>("missing_checks").0,
        gate_reasons: row
            .get::<Option<Json<Vec<String>>>, _>("gate_reasons")
            .map(|reasons| reasons.0)
            .unwrap_or_default(),
        merge_queue: queue_from(row, row.get("landed")),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}

/// The merge-queue state a row records; landed whatever it recorded once
/// the pull request's merge is.
fn queue_from(row: &sqlx::sqlite::SqliteRow, landed: bool) -> Option<MergeQueue> {
    if landed {
        return Some(MergeQueue {
            state: MergeQueueState::Landed,
            reason: None,
        });
    }
    let state: Option<String> = row.get("merge_queue");
    Some(MergeQueue {
        state: MergeQueueState::parse(&state?)?,
        reason: row.get("merge_queue_reason"),
    })
}
