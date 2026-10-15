//! Swarm ramp metrics on Postgres; the computation is in
//! [`maidan_types::change_metrics`].

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use maidan_fsm::ThreadAction;
use maidan_types::{
    ChangePull, ChangeRound, ChangeSettlement, Event, MemberId, NewChangeRound, StoredEvent,
    ThreadId, ThreadState, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{events, thread_transitions};
use crate::error::StoreError;

/// Record a delivered round, and the pull request the first time. A retried
/// delivery of the same commit adds nothing.
pub async fn record_round(pool: &PgPool, new: NewChangeRound) -> Result<(), StoreError> {
    let repo = new.repo.to_ascii_lowercase();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO maidan_change_pulls (repo, pull_number, workspace_id, thread_id, opened_at)
         VALUES ($1, $2, $3, $4, NOW())
         ON CONFLICT (repo, pull_number) DO NOTHING",
    )
    .bind(&repo)
    .bind(new.pull_number)
    .bind(new.workspace_id.0)
    .bind(new.thread_id.0)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO maidan_change_rounds
             (repo, pull_number, commit_sha, round, base_sha, base_behind, diff_lines, delivered_at)
         SELECT $1, $2, $3, COALESCE(MAX(round), 0) + 1, $4, $5, $6, NOW()
         FROM maidan_change_rounds WHERE repo = $1 AND pull_number = $2
         ON CONFLICT (repo, pull_number, commit_sha) DO NOTHING",
    )
    .bind(&repo)
    .bind(new.pull_number)
    .bind(new.commit_sha.to_ascii_lowercase())
    .bind(&new.base_sha)
    .bind(new.base_behind)
    .bind(new.diff_lines)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Stamp when a tracked pull request was made ready (the first time only).
pub async fn mark_ready(
    pool: &PgPool,
    repo: &str,
    pull_number: i64,
    at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_change_pulls SET ready_at = COALESCE(ready_at, $3)
         WHERE repo = $1 AND pull_number = $2",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record that a tracked pull request closed, merged or not.
pub async fn close(
    pool: &PgPool,
    repo: &str,
    pull_number: i64,
    merged_at: Option<DateTime<Utc>>,
    merged_head_sha: Option<&str>,
    closed_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_change_pulls
         SET merged_at = $3, merged_head_sha = $4, closed_at = $5
         WHERE repo = $1 AND pull_number = $2 AND closed_at IS NULL",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(merged_at)
    .bind(merged_head_sha.map(str::to_ascii_lowercase))
    .bind(closed_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// The tracked pull requests in `workspace_id` not yet seen closed, at most
/// `limit`: the longest unread first, never read before all, so a workspace
/// with more than `limit` is read in full over a few passes.
pub async fn list_open(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<(String, i64)>, StoreError> {
    let rows = sqlx::query(
        "SELECT repo, pull_number FROM maidan_change_pulls
         WHERE workspace_id = $1 AND closed_at IS NULL
         ORDER BY checked_at ASC NULLS FIRST, opened_at, repo, pull_number
         LIMIT $2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get("repo"), row.get("pull_number")))
        .collect())
}

/// Stamp when a tracked pull request was last read from GitHub.
pub async fn mark_checked(
    pool: &PgPool,
    repo: &str,
    pull_number: i64,
    at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_change_pulls SET checked_at = $3
         WHERE repo = $1 AND pull_number = $2",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(at)
    .execute(pool)
    .await?;
    Ok(())
}

/// The workspaces with a tracked pull request still to read or to settle.
pub async fn list_sweep_workspaces(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<WorkspaceId>, StoreError> {
    let rows = sqlx::query(
        "SELECT DISTINCT workspace_id FROM maidan_change_pulls
         WHERE closed_at IS NULL OR settled_at IS NULL
         ORDER BY workspace_id LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| WorkspaceId(row.get("workspace_id")))
        .collect())
}

/// The tracked pull requests in `workspace_id` seen closed and not yet
/// settled, oldest close first, at most `limit`.
pub async fn list_unsettled(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<(String, i64)>, StoreError> {
    let rows = sqlx::query(
        "SELECT repo, pull_number FROM maidan_change_pulls
         WHERE workspace_id = $1 AND closed_at IS NOT NULL AND settled_at IS NULL
         ORDER BY closed_at, repo, pull_number
         LIMIT $2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get("repo"), row.get("pull_number")))
        .collect())
}

/// Settle a tracked pull request seen closed, once. A merge lands its thread:
/// closed through the transition path, with `ThreadLanded` in the same
/// transaction. A close without a merge leaves the thread open and owes one
/// notice. See [`ChangeSettlement`].
pub async fn settle(
    pool: &PgPool,
    repo: &str,
    pull_number: i64,
) -> Result<ChangeSettlement, StoreError> {
    let repo = repo.to_ascii_lowercase();
    let mut tx = pool.begin().await?;
    let Some(row) = sqlx::query(
        "SELECT p.thread_id, p.merged_at IS NOT NULL AS merged, t.state::text AS state,
                t.tombstoned_at IS NOT NULL AS tombstoned, t.owner_id, r.produced_by
         FROM maidan_change_pulls p
         JOIN maidan_threads t ON t.id = p.thread_id
         LEFT JOIN maidan_thread_results r ON r.thread_id = p.thread_id
         WHERE p.repo = $1 AND p.pull_number = $2
           AND p.closed_at IS NOT NULL AND p.settled_at IS NULL
         FOR UPDATE OF p",
    )
    .bind(&repo)
    .bind(pull_number)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(ChangeSettlement::Unchanged);
    };
    // Claimed before acting, so two replicas sweeping at once settle it once.
    if mark_settled(&mut *tx, &repo, pull_number).await? == 0 {
        return Ok(ChangeSettlement::Unchanged);
    }
    let thread_id = ThreadId(row.get("thread_id"));
    // The owner first, so separation of duties never refuses the landing; a
    // thread without one is landed by whoever produced the delivered result.
    let actor = row
        .get::<Option<Uuid>, _>("owner_id")
        .or_else(|| row.get::<Option<Uuid>, _>("produced_by"))
        .map(MemberId);
    let state: String = row.get("state");
    let from_open = state == ThreadState::Open.as_str();
    let live = from_open || state == ThreadState::InReview.as_str();
    let (Some(actor), true, false) = (actor, live, row.get::<bool, _>("tombstoned")) else {
        tx.commit().await?;
        return Ok(ChangeSettlement::Unchanged);
    };
    if !row.get::<bool, _>("merged") {
        tx.commit().await?;
        return Ok(ChangeSettlement::ClosedUnmerged {
            thread_id,
            author_id: actor,
        });
    }
    let another_open = sqlx::query(
        "SELECT 1 FROM maidan_change_pulls WHERE thread_id = $1 AND closed_at IS NULL LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if another_open {
        tx.commit().await?;
        return Ok(ChangeSettlement::Unchanged);
    }
    match land_in_tx(&mut tx, thread_id, actor, from_open, &repo, pull_number).await {
        Ok(stored) => {
            tx.commit().await?;
            Ok(ChangeSettlement::Landed(stored))
        }
        Err(StoreError::Conflict(reason)) => {
            tx.rollback().await?;
            // The merge stands whatever the thread's gates say, so the close
            // is settled now and the refusal said once, not retried each tick.
            let mut tx = pool.begin().await?;
            let claimed = mark_settled(&mut *tx, &repo, pull_number).await?;
            tx.commit().await?;
            if claimed == 0 {
                return Ok(ChangeSettlement::Unchanged);
            }
            Ok(ChangeSettlement::CloseRefused {
                thread_id,
                author_id: actor,
                reason,
            })
        }
        Err(err) => Err(err),
    }
}

async fn mark_settled(
    conn: &mut <sqlx::Postgres as sqlx::Database>::Connection,
    repo: &str,
    pull_number: i64,
) -> Result<u64, StoreError> {
    Ok(sqlx::query(
        "UPDATE maidan_change_pulls SET settled_at = $3
         WHERE repo = $1 AND pull_number = $2 AND settled_at IS NULL",
    )
    .bind(repo)
    .bind(pull_number)
    .bind(Utc::now())
    .execute(conn)
    .await?
    .rows_affected())
}

async fn land_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    actor: MemberId,
    from_open: bool,
    repo: &str,
    pull_number: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut stored = Vec::new();
    // The thread machine has no edge from open to closed. A merged change was
    // reviewed on GitHub, so it passes through review on its way, and the
    // review and close gates still apply.
    if from_open {
        let review =
            thread_transitions::transition_in_tx(tx, thread_id, actor, ThreadAction::StartReview)
                .await?;
        stored.push(thread_transitions::state_changed_in_tx(tx, actor, &review).await?);
    }
    let closed =
        thread_transitions::transition_in_tx(tx, thread_id, actor, ThreadAction::Close).await?;
    stored.push(thread_transitions::state_changed_in_tx(tx, actor, &closed).await?);
    let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, thread_id).await?;
    let landed = Event::ThreadLanded {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id,
        repo: repo.to_string(),
        pr_number: pull_number,
        merged_by: None,
        merge_commit_sha: None,
        title: None,
    };
    stored.push(events::append_in_tx(tx, &landed).await?);
    Ok(stored)
}

/// The tracked pull requests in `workspace_id`, in `repo` and opened since
/// `since` when given, oldest first, each with its rounds and its thread's
/// model cost.
pub async fn list(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: Option<&str>,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<ChangePull>, StoreError> {
    let repo = repo.map(str::to_ascii_lowercase);
    let pulls = sqlx::query(
        "SELECT repo, pull_number, thread_id, opened_at, ready_at, merged_at, merged_head_sha, closed_at
         FROM maidan_change_pulls
         WHERE workspace_id = $1
           AND ($2::text IS NULL OR repo = $2)
           AND ($3::timestamptz IS NULL OR opened_at >= $3)
         ORDER BY opened_at, repo, pull_number
         LIMIT 1000",
    )
    .bind(workspace_id.0)
    .bind(&repo)
    .bind(since)
    .fetch_all(pool)
    .await?;
    let rounds = sqlx::query(
        "SELECT r.repo, r.pull_number, r.commit_sha, r.round, r.base_behind, r.diff_lines, r.delivered_at
         FROM maidan_change_rounds r
         JOIN maidan_change_pulls p ON p.repo = r.repo AND p.pull_number = r.pull_number
         WHERE p.workspace_id = $1
         ORDER BY r.repo, r.pull_number, r.round",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    let mut by_pull: HashMap<(String, i64), Vec<ChangeRound>> = HashMap::new();
    for row in &rounds {
        by_pull
            .entry((row.get("repo"), row.get("pull_number")))
            .or_default()
            .push(ChangeRound {
                round: row.get("round"),
                commit_sha: row.get("commit_sha"),
                base_behind: row.get("base_behind"),
                diff_lines: row.get("diff_lines"),
                delivered_at: row.get::<DateTime<Utc>, _>("delivered_at"),
            });
    }
    let costs = sqlx::query(
        "SELECT thread_id, COALESCE(SUM(usd_micros), 0) AS cost
         FROM maidan_usage_ledger WHERE workspace_id = $1
         GROUP BY thread_id",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    let cost: HashMap<Uuid, i64> = costs
        .iter()
        .map(|row| (row.get("thread_id"), row.get("cost")))
        .collect();
    Ok(pulls
        .iter()
        .map(|row| pull_from(row, &mut by_pull, &cost))
        .collect())
}

fn pull_from(
    row: &sqlx::postgres::PgRow,
    rounds: &mut HashMap<(String, i64), Vec<ChangeRound>>,
    cost: &HashMap<Uuid, i64>,
) -> ChangePull {
    let repo: String = row.get("repo");
    let pull_number: i64 = row.get("pull_number");
    let thread: Uuid = row.get("thread_id");
    ChangePull {
        thread_id: ThreadId(thread),
        rounds: rounds
            .remove(&(repo.clone(), pull_number))
            .unwrap_or_default(),
        repo,
        pull_number,
        opened_at: row.get::<DateTime<Utc>, _>("opened_at"),
        ready_at: row.get::<Option<DateTime<Utc>>, _>("ready_at"),
        merged_at: row.get::<Option<DateTime<Utc>>, _>("merged_at"),
        merged_head_sha: row.get("merged_head_sha"),
        closed_at: row.get::<Option<DateTime<Utc>>, _>("closed_at"),
        cost_usd_micros: cost.get(&thread).copied().unwrap_or(0),
    }
}
