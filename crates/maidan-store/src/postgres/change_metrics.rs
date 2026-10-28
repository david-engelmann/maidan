//! Swarm ramp metrics on Postgres; the computation is in
//! [`maidan_types::change_metrics`].

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use maidan_fsm::ThreadAction;
use maidan_types::{
    ChangePull, ChangeRound, ChangeSettlement, ContentKeyring, Event, MemberId, MergedOutcome,
    NewChangeRound, NewDeploy, NewStaleBaseNotice, OutcomeDue, PostedNotice, StoredEvent, ThreadId,
    ThreadNotice, ThreadNoticeKind, ThreadState, WorkspaceId, NOTICE_PATH_LIMIT,
    OUTCOME_WINDOW_DAYS,
};
use sqlx::types::Json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{events, thread_transitions};
use crate::change_notices::merged_paths;
use crate::error::StoreError;

/// Record a delivered round, and the pull request the first time. A retried
/// delivery of the same commit adds nothing.
pub async fn record_round(pool: &PgPool, new: NewChangeRound) -> Result<(), StoreError> {
    let repo = new.repo.to_ascii_lowercase();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO maidan_change_pulls (repo, pull_number, workspace_id, thread_id, opened_at, linear_issue)
         VALUES ($1, $2, $3, $4, NOW(), $5)
         ON CONFLICT (workspace_id, repo, pull_number) DO UPDATE
             SET linear_issue = COALESCE(maidan_change_pulls.linear_issue, excluded.linear_issue)",
    )
    .bind(&repo)
    .bind(new.pull_number)
    .bind(new.workspace_id.0)
    .bind(new.thread_id.0)
    .bind(&new.linear_issue)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO maidan_change_rounds
             (repo, pull_number, commit_sha, round, base_sha, base_behind, diff_lines, delivered_at,
              paths, base_branch, rebase, workspace_id, previous_head)
         SELECT $1, $2, $3, COALESCE(MAX(round), 0) + 1, $4, $5, $6, NOW(), $7, $8, $9, $10, $11
         FROM maidan_change_rounds WHERE workspace_id = $10 AND repo = $1 AND pull_number = $2
         ON CONFLICT (workspace_id, repo, pull_number, commit_sha) DO NOTHING",
    )
    .bind(&repo)
    .bind(new.pull_number)
    .bind(new.commit_sha.to_ascii_lowercase())
    .bind(&new.base_sha)
    .bind(new.base_behind)
    .bind(new.diff_lines)
    .bind(Json(&new.paths))
    .bind(&new.base_branch)
    .bind(new.rebase)
    .bind(new.workspace_id.0)
    .bind(new.previous_head.as_deref().map(str::to_ascii_lowercase))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The commit of the newest round the change flow recorded for this pull
/// request in `workspace_id`: what the agent branch's head must still be
/// before a squash-rebase may force-move it. Only rounds this workspace
/// delivered count, so another workspace's commit on the same pull request
/// never vouches for the branch.
pub async fn latest_round_head(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
) -> Result<Option<String>, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT r.commit_sha FROM maidan_change_rounds r
         JOIN maidan_change_pulls p
           ON p.workspace_id = r.workspace_id AND p.repo = r.repo AND p.pull_number = r.pull_number
         WHERE p.workspace_id = $1 AND r.workspace_id = $1
           AND r.repo = $2 AND r.pull_number = $3
         ORDER BY r.round DESC
         LIMIT 1",
    )
    .bind(workspace_id.0)
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .fetch_optional(pool)
    .await?)
}

/// The head the squash-rebase round that delivered `commit_sha` to
/// `repo`#`pull_number` force-moved the branch from, as recorded in
/// `workspace_id`. `None` for a normal round, an unrecorded one, or another
/// workspace's.
pub async fn round_previous_head(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
    commit_sha: &str,
) -> Result<Option<String>, StoreError> {
    let head: Option<Option<String>> = sqlx::query_scalar(
        "SELECT r.previous_head FROM maidan_change_rounds r
         JOIN maidan_change_pulls p
           ON p.workspace_id = r.workspace_id AND p.repo = r.repo AND p.pull_number = r.pull_number
         WHERE p.workspace_id = $1 AND r.workspace_id = $1
           AND r.repo = $2 AND r.pull_number = $3 AND r.commit_sha = $4",
    )
    .bind(workspace_id.0)
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(commit_sha.to_ascii_lowercase())
    .fetch_optional(pool)
    .await?;
    Ok(head.flatten())
}

/// Stamp when a tracked pull request was made ready (the first time only).
pub async fn mark_ready(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
    at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_change_pulls SET ready_at = COALESCE(ready_at, $3)
         WHERE repo = $1 AND pull_number = $2 AND workspace_id = $4",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(at)
    .bind(workspace_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record that a tracked pull request closed, merged or not.
pub async fn close(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
    merged_at: Option<DateTime<Utc>>,
    merged_head_sha: Option<&str>,
    closed_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_change_pulls
         SET merged_at = $3, merged_head_sha = $4, closed_at = $5
         WHERE repo = $1 AND pull_number = $2 AND workspace_id = $6 AND closed_at IS NULL",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(merged_at)
    .bind(merged_head_sha.map(str::to_ascii_lowercase))
    .bind(closed_at)
    .bind(workspace_id.0)
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
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
    at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_change_pulls SET checked_at = $3
         WHERE repo = $1 AND pull_number = $2 AND workspace_id = $4",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(at)
    .bind(workspace_id.0)
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
            OR (merged_at IS NOT NULL AND noticed_at IS NULL)
            OR (merged_at >= $2 AND (outcome_checked_at IS NULL OR outcome_checked_at <= $3))
         ORDER BY workspace_id LIMIT $1",
    )
    .bind(limit)
    // A merge inside its outcome window, due its daily check.
    .bind(Utc::now() - chrono::Duration::days(OUTCOME_WINDOW_DAYS))
    .bind(Utc::now() - chrono::Duration::days(1))
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
    keys: &ContentKeyring,
    workspace_id: WorkspaceId,
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
         WHERE p.repo = $1 AND p.pull_number = $2 AND p.workspace_id = $3
           AND p.closed_at IS NOT NULL AND p.settled_at IS NULL
         FOR UPDATE OF p",
    )
    .bind(&repo)
    .bind(pull_number)
    .bind(workspace_id.0)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(ChangeSettlement::Unchanged);
    };
    // Claimed before acting, so two replicas sweeping at once settle it once.
    if mark_settled(&mut tx, workspace_id, &repo, pull_number).await? == 0 {
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
    let another_open = sqlx::query(
        "SELECT 1 FROM maidan_change_pulls WHERE thread_id = $1 AND closed_at IS NULL LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    // The thread lands once none of its pull requests is still open, on its
    // latest merge, whichever of them closed last: a merge that waited on a
    // sibling is not lost when the sibling then closes without merging.
    let merged = if another_open {
        None
    } else {
        sqlx::query(
            "SELECT repo, pull_number FROM maidan_change_pulls
             WHERE thread_id = $1 AND merged_at IS NOT NULL
             ORDER BY merged_at DESC, repo, pull_number LIMIT 1",
        )
        .bind(thread_id.0)
        .fetch_optional(&mut *tx)
        .await?
        .map(|row| {
            (
                row.get::<String, _>("repo"),
                row.get::<i64, _>("pull_number"),
            )
        })
    };
    let Some((landed_repo, landed_pull)) = merged else {
        tx.commit().await?;
        return Ok(if row.get::<bool, _>("merged") {
            ChangeSettlement::Unchanged
        } else {
            ChangeSettlement::ClosedUnmerged {
                thread_id,
                author_id: actor,
            }
        });
    };
    match land_in_tx(
        &mut tx,
        keys,
        thread_id,
        actor,
        from_open,
        &landed_repo,
        landed_pull,
    )
    .await
    {
        Ok(stored) => {
            tx.commit().await?;
            Ok(ChangeSettlement::Landed(stored))
        }
        Err(StoreError::Conflict(reason)) => {
            tx.rollback().await?;
            // The merge stands whatever the thread's gates say, so the close
            // is settled now and the refusal said once, not retried each tick.
            let mut tx = pool.begin().await?;
            let claimed = mark_settled(&mut tx, workspace_id, &repo, pull_number).await?;
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
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
) -> Result<u64, StoreError> {
    Ok(sqlx::query(
        "UPDATE maidan_change_pulls SET settled_at = $3
         WHERE repo = $1 AND pull_number = $2 AND workspace_id = $4 AND settled_at IS NULL",
    )
    .bind(repo)
    .bind(pull_number)
    .bind(Utc::now())
    .bind(workspace_id.0)
    .execute(conn)
    .await?
    .rows_affected())
}

async fn land_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    keys: &ContentKeyring,
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
    stored.extend(super::plans::after_close_in_tx(tx, keys, thread_id, actor).await?);
    Ok(stored)
}

/// Notify the merges in `workspace_id` not yet noticed, oldest merge first,
/// at most `limit`: each live claim in the same workspace and repository
/// whose footprint covers a path the merged pull request touched gets one
/// `rebase_needed` notice. See [`notify_merge`].
pub async fn notify_merges(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<PostedNotice>, StoreError> {
    let rows = sqlx::query(
        "SELECT repo, pull_number FROM maidan_change_pulls
         WHERE workspace_id = $1 AND merged_at IS NOT NULL AND noticed_at IS NULL
         ORDER BY merged_at, repo, pull_number
         LIMIT $2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut posted = Vec::new();
    for row in &rows {
        let repo: String = row.get("repo");
        posted.extend(notify_merge(pool, workspace_id, &repo, row.get("pull_number")).await?);
    }
    Ok(posted)
}

/// Notify one merge, once: the pull request is stamped noticed in the same
/// transaction that records its notices, so two replicas sweeping at once
/// notify it once, and a failure leaves it for the next pass.
async fn notify_merge(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
) -> Result<Vec<PostedNotice>, StoreError> {
    let mut tx = pool.begin().await?;
    let claimed = sqlx::query(
        "UPDATE maidan_change_pulls SET noticed_at = $3
         WHERE repo = $1 AND pull_number = $2 AND workspace_id = $4
           AND merged_at IS NOT NULL AND noticed_at IS NULL",
    )
    .bind(repo)
    .bind(pull_number)
    .bind(Utc::now())
    .bind(workspace_id.0)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if claimed == 0 {
        return Ok(Vec::new());
    }
    let own_thread: Uuid = sqlx::query_scalar(
        "SELECT thread_id FROM maidan_change_pulls
         WHERE repo = $1 AND pull_number = $2 AND workspace_id = $3",
    )
    .bind(repo)
    .bind(pull_number)
    .bind(workspace_id.0)
    .fetch_one(&mut *tx)
    .await?;
    let rounds = sqlx::query(
        "SELECT paths, base_branch FROM maidan_change_rounds
         WHERE repo = $1 AND pull_number = $2 AND workspace_id = $3 ORDER BY round",
    )
    .bind(repo)
    .bind(pull_number)
    .bind(workspace_id.0)
    .fetch_all(&mut *tx)
    .await?;
    let (paths, base_branch) = merged_paths(rounds.iter().map(|row| {
        (
            row.get::<Json<Vec<String>>, _>("paths").0,
            row.get::<Option<String>, _>("base_branch"),
        )
    }));
    if paths.is_empty() {
        tx.commit().await?;
        return Ok(Vec::new());
    }
    let keys = sqlx::query(&crate::footprints::live_path_keys_select(
        "$1", "$2", "$3", "NOW()",
    ))
    .bind(workspace_id.0)
    .bind(repo)
    .bind(own_thread)
    .fetch_all(&mut *tx)
    .await?
    .iter()
    .map(|row| (row.get("thread_id"), row.get("match_key")))
    .collect();
    let mut posted = Vec::new();
    for (thread, keys) in crate::footprints::keys_by_thread(keys) {
        let covered = crate::footprints::covered_paths(&keys, &paths, NOTICE_PATH_LIMIT);
        if covered.is_empty() {
            continue;
        }
        let notice = ThreadNotice {
            kind: ThreadNoticeKind::RebaseNeeded,
            repo: repo.to_string(),
            pull_number,
            paths: covered,
            base_branch: base_branch.clone(),
            base_behind: None,
            head_sha: None,
            status: None,
            failing_jobs: Vec::new(),
            reasons: Vec::new(),
            environment: None,
            deploy_url: None,
            created_at: Utc::now(),
        };
        let key = format!("rebase_needed:{repo}#{pull_number}");
        if let Some(p) = insert_notice_in_tx(&mut tx, ThreadId(thread), notice, &key).await? {
            posted.push(p);
        }
    }
    tx.commit().await?;
    Ok(posted)
}

/// Record a stale-base notice on the thread a round was delivered on, once
/// per delivered commit. `None` when it was already given.
pub async fn add_stale_base_notice(
    pool: &PgPool,
    new: NewStaleBaseNotice,
) -> Result<Option<PostedNotice>, StoreError> {
    let repo = new.repo.to_ascii_lowercase();
    let key = format!(
        "stale_base:{repo}#{}@{}",
        new.pull_number,
        new.commit_sha.to_ascii_lowercase()
    );
    let notice = ThreadNotice {
        kind: ThreadNoticeKind::StaleBase,
        repo,
        pull_number: new.pull_number,
        paths: Vec::new(),
        base_branch: new.base_branch,
        base_behind: Some(new.base_behind),
        head_sha: None,
        status: None,
        failing_jobs: Vec::new(),
        reasons: Vec::new(),
        environment: None,
        deploy_url: None,
        created_at: Utc::now(),
    };
    let mut tx = pool.begin().await?;
    let posted = insert_notice_in_tx(&mut tx, new.thread_id, notice, &key).await?;
    tx.commit().await?;
    Ok(posted)
}

/// Record a deploy on every merged pull request whose merge (or merged head)
/// commit it names. The deploy row and the notice commit together, so a
/// retried hook call finds both and says nothing twice.
pub async fn record_deploy(
    pool: &PgPool,
    deploy: NewDeploy,
) -> Result<Vec<PostedNotice>, StoreError> {
    let repo = deploy.repo.to_ascii_lowercase();
    let sha = deploy.sha.to_ascii_lowercase();
    let mut tx = pool.begin().await?;
    // Every workspace that tracks the pull request hears of it: the hook
    // names a commit, and each tracking workspace merged that commit.
    let matched = sqlx::query(
        "SELECT workspace_id, pull_number, thread_id FROM maidan_change_pulls
         WHERE repo = $1 AND merged_at IS NOT NULL
           AND (LOWER(merge_commit_sha) = $2 OR LOWER(merged_head_sha) = $2)
         ORDER BY pull_number",
    )
    .bind(&repo)
    .bind(&sha)
    .fetch_all(&mut *tx)
    .await?;
    let mut targets: Vec<(Uuid, i64, Uuid)> = matched
        .iter()
        .map(|row| {
            (
                row.get("workspace_id"),
                row.get("pull_number"),
                row.get("thread_id"),
            )
        })
        .collect();
    // A pull the deployed commit carries as an ancestor, looked up in the
    // workspace that tracks it and no other.
    for pull in &deploy.contains {
        let row = sqlx::query(
            "SELECT thread_id FROM maidan_change_pulls
             WHERE workspace_id = $1 AND repo = $2 AND pull_number = $3 AND merged_at IS NOT NULL",
        )
        .bind(pull.workspace_id.0)
        .bind(&repo)
        .bind(pull.pull_number)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = row {
            let target = (pull.workspace_id.0, pull.pull_number, row.get("thread_id"));
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
    }
    let mut posted = Vec::new();
    for (workspace_id, pull_number, thread_id) in targets {
        let thread_id = ThreadId(thread_id);
        sqlx::query(
            "INSERT INTO maidan_deploys
                 (id, workspace_id, thread_id, repo, pull_number, sha, environment, status, url, received_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (workspace_id, repo, pull_number, sha, environment, status) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(workspace_id)
        .bind(thread_id.0)
        .bind(&repo)
        .bind(pull_number)
        .bind(&sha)
        .bind(&deploy.environment)
        .bind(deploy.status.as_str())
        .bind(&deploy.url)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;
        let key = format!(
            "deployed:{repo}#{pull_number}@{sha}:{}:{}",
            deploy.environment,
            deploy.status.as_str()
        );
        let notice = ThreadNotice {
            kind: ThreadNoticeKind::Deployed,
            repo: repo.clone(),
            pull_number,
            paths: Vec::new(),
            base_branch: None,
            base_behind: None,
            head_sha: Some(sha.clone()),
            status: Some(deploy.status),
            failing_jobs: Vec::new(),
            reasons: Vec::new(),
            environment: Some(deploy.environment.clone()),
            deploy_url: deploy.url.clone(),
            created_at: Utc::now(),
        };
        if let Some(one) = insert_notice_in_tx(&mut tx, thread_id, notice, &key).await? {
            posted.push(one);
        }
    }
    tx.commit().await?;
    Ok(posted)
}

/// See [`crate::Store::deploy_candidates`].
pub async fn deploy_candidates(
    pool: &PgPool,
    repo: &str,
    environment: &str,
    limit: i64,
) -> Result<Vec<maidan_types::DeployCandidate>, StoreError> {
    let rows = sqlx::query(
        "SELECT p.workspace_id, p.pull_number,
                LOWER(COALESCE(p.merge_commit_sha, p.merged_head_sha)) AS merge_sha
         FROM maidan_change_pulls p
         WHERE p.repo = $1 AND p.merged_at IS NOT NULL
           AND COALESCE(p.merge_commit_sha, p.merged_head_sha) IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM maidan_deploys d
               WHERE d.workspace_id = p.workspace_id AND d.repo = p.repo
                 AND d.pull_number = p.pull_number AND d.environment = $2
                 AND d.status = 'success')
         ORDER BY p.merged_at DESC, p.pull_number DESC
         LIMIT $3",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(environment)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| maidan_types::DeployCandidate {
            workspace_id: WorkspaceId(row.get("workspace_id")),
            pull_number: row.get("pull_number"),
            merge_sha: row.get("merge_sha"),
        })
        .collect())
}

pub(super) async fn insert_notice_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    notice: ThreadNotice,
    dedupe_key: &str,
) -> Result<Option<PostedNotice>, StoreError> {
    let (workspace_id, channel_id) = events::thread_scope_in_tx(tx, thread_id).await?;
    let inserted = sqlx::query(
        "INSERT INTO maidan_thread_notices
             (id, workspace_id, thread_id, kind, repo, pull_number, paths, base_branch,
              base_behind, dedupe_key, created_at, head_sha, status, failing_jobs, reasons,
              environment, deploy_url)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
         ON CONFLICT (thread_id, dedupe_key) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(workspace_id.0)
    .bind(thread_id.0)
    .bind(notice.kind.as_str())
    .bind(&notice.repo)
    .bind(notice.pull_number)
    .bind(Json(&notice.paths))
    .bind(&notice.base_branch)
    .bind(notice.base_behind)
    .bind(dedupe_key)
    .bind(notice.created_at)
    .bind(&notice.head_sha)
    .bind(notice.status.map(|status| status.as_str()))
    .bind(Json(&notice.failing_jobs))
    .bind(Json(&notice.reasons))
    .bind(&notice.environment)
    .bind(&notice.deploy_url)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(None);
    }
    let event = events::append_in_tx(
        tx,
        &Event::ThreadNotice {
            occurred_at: notice.created_at,
            workspace_id,
            channel_id,
            thread_id,
            notice: notice.kind,
            repo: notice.repo.clone(),
            pr_number: notice.pull_number,
            paths: notice.paths.clone(),
            base_branch: notice.base_branch.clone(),
            base_behind: notice.base_behind,
            head_sha: notice.head_sha.clone(),
            status: notice.status,
            failing_jobs: notice.failing_jobs.clone(),
            reasons: notice.reasons.clone(),
            environment: notice.environment.clone(),
            deploy_url: notice.deploy_url.clone(),
        },
    )
    .await?;
    let author_id = sqlx::query(
        "SELECT COALESCE(t.owner_id, t.assignee_id, r.produced_by) AS author
         FROM maidan_threads t
         LEFT JOIN maidan_thread_results r ON r.thread_id = t.id
         WHERE t.id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .and_then(|row| row.get::<Option<Uuid>, _>("author"))
    .map(MemberId);
    Ok(Some(PostedNotice {
        thread_id,
        notice,
        event,
        author_id,
    }))
}

/// A thread's notices, newest first, at most `limit`.
pub async fn list_notices(
    pool: &PgPool,
    thread_id: ThreadId,
    limit: i64,
) -> Result<Vec<ThreadNotice>, StoreError> {
    let rows = sqlx::query(
        "SELECT kind, repo, pull_number, paths, base_branch, base_behind, created_at,
                head_sha, status, failing_jobs, reasons, environment, deploy_url
         FROM maidan_thread_notices WHERE thread_id = $1
         ORDER BY created_at DESC, id DESC LIMIT $2",
    )
    .bind(thread_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            let kind: String = row.get("kind");
            Ok(ThreadNotice {
                kind: ThreadNoticeKind::parse(&kind).ok_or_else(|| {
                    StoreError::InvalidInput(format!("unknown notice kind `{kind}`"))
                })?,
                repo: row.get("repo"),
                pull_number: row.get("pull_number"),
                paths: row.get::<Json<Vec<String>>, _>("paths").0,
                base_branch: row.get("base_branch"),
                base_behind: row.get("base_behind"),
                head_sha: row.get("head_sha"),
                status: row
                    .get::<Option<String>, _>("status")
                    .and_then(|status| maidan_types::CiStatus::parse(&status)),
                failing_jobs: row.get::<Json<Vec<String>>, _>("failing_jobs").0,
                reasons: row.get::<Json<Vec<String>>, _>("reasons").0,
                environment: row.get("environment"),
                deploy_url: row.get("deploy_url"),
                created_at: row.get::<DateTime<Utc>, _>("created_at"),
            })
        })
        .collect()
}

/// The tracked pull requests in `workspace_id`, in `repo` and opened since
/// `since` when given, oldest first, each with its rounds and its thread's
/// model cost.
pub async fn list(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: Option<&str>,
    since: Option<DateTime<Utc>>,
    engine: Option<maidan_types::TaskEngine>,
    kind: Option<maidan_types::TaskKind>,
    size: Option<maidan_types::TaskSize>,
) -> Result<Vec<ChangePull>, StoreError> {
    let repo = repo.map(str::to_ascii_lowercase);
    let pulls = sqlx::query(
        "SELECT p.repo, p.pull_number, p.thread_id, p.opened_at, p.ready_at, p.merged_at, p.merged_head_sha, p.closed_at, p.linear_issue,
                (SELECT h.handed_off_at FROM maidan_thread_handoffs h WHERE h.thread_id = p.thread_id) AS handed_off_at,
                p.merge_commit_sha, p.reverted_at, p.revert_ref, p.followup_fixes, p.human_commits,
                p.human_reviews, p.first_human_review_at,
                a.kind AS task_kind, a.engine AS task_engine, a.threat_model AS threat_model, a.size AS task_size, a.acceptance AS task_acceptance, a.linear_issue AS task_linear_issue, a.tier AS task_tier, a.lane AS task_lane
         FROM maidan_change_pulls p
         LEFT JOIN maidan_thread_attributes a ON a.thread_id = p.thread_id
         WHERE p.workspace_id = $1 AND NOT p.watch_only
           AND ($2::text IS NULL OR p.repo = $2)
           AND ($3::timestamptz IS NULL OR p.opened_at >= $3)
           AND ($4::text IS NULL OR a.engine = $4)
           AND ($5::text IS NULL OR COALESCE(a.kind, 'change') = $5)
           AND ($6::text IS NULL OR a.size = $6)
         ORDER BY p.opened_at, p.repo, p.pull_number
         LIMIT 1000",
    )
    .bind(workspace_id.0)
    .bind(&repo)
    .bind(since)
    .bind(engine.map(|engine| engine.as_str()))
    .bind(kind.map(|kind| kind.as_str()))
    .bind(size.map(|size| size.as_str()))
    .fetch_all(pool)
    .await?;
    let rounds = sqlx::query(
        "SELECT r.repo, r.pull_number, r.commit_sha, r.round, r.base_behind, r.diff_lines, r.delivered_at,
                r.rebase, r.previous_head
         FROM maidan_change_rounds r
         JOIN maidan_change_pulls p
           ON p.workspace_id = r.workspace_id AND p.repo = r.repo AND p.pull_number = r.pull_number
         WHERE p.workspace_id = $1 AND r.workspace_id = $1
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
                rebase: row.get::<Option<bool>, _>("rebase").unwrap_or(false),
                previous_head: row.get("previous_head"),
            });
    }
    let spend_rows = sqlx::query(
        "SELECT thread_id, provider, harness,
                COALESCE(SUM(usd_micros), 0)::BIGINT AS cost, COUNT(*)::BIGINT AS reports
         FROM maidan_usage_ledger WHERE workspace_id = $1
         GROUP BY thread_id, provider, harness",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    let cost = maidan_types::thread_spend(spend_rows.iter().map(|row| maidan_types::SpendRow {
        thread_id: ThreadId(row.get("thread_id")),
        provider: row.get("provider"),
        harness: row.get("harness"),
        usd_micros: row.get("cost"),
        reports: row.get("reports"),
    }));
    let mut pulls: Vec<ChangePull> = pulls
        .iter()
        .map(|row| pull_from(row, &mut by_pull, &cost))
        .collect();
    let flows = sqlx::query(
        "SELECT e.thread_id,
                MIN(CASE WHEN e.kind = 'thread_assignment_changed'
                          AND e.payload->>'assignee_id' IS NOT NULL
                         THEN e.occurred_at END) AS claimed_at,
                MIN(CASE WHEN e.kind = 'thread_state_changed'
                          AND e.payload->>'to_state' = 'in_review'
                         THEN e.occurred_at END) AS review_requested_at,
                MIN(CASE WHEN e.kind = 'review_submitted' THEN e.occurred_at END) AS first_review_at,
                COALESCE(SUM(CASE WHEN e.kind = 'review_submitted'
                          AND e.payload->>'decision' = 'request_changes'
                         THEN 1 ELSE 0 END), 0)::BIGINT AS sent_back,
                COALESCE(SUM(CASE WHEN e.kind = 'claim_expired' THEN 1 ELSE 0 END), 0)::BIGINT AS claim_lapses
         FROM maidan_events e
         WHERE e.workspace_id = $1
           AND e.kind IN ('thread_assignment_changed', 'thread_state_changed', 'review_submitted', 'claim_expired')
           AND e.thread_id IN (SELECT p.thread_id FROM maidan_change_pulls p WHERE p.workspace_id = $1)
         GROUP BY e.thread_id",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    let mut flow: HashMap<Uuid, maidan_types::ThreadFlow> = flows
        .iter()
        .map(|row| {
            (
                row.get("thread_id"),
                maidan_types::ThreadFlow {
                    claimed_at: row.get::<Option<DateTime<Utc>>, _>("claimed_at"),
                    review_requested_at: row.get::<Option<DateTime<Utc>>, _>("review_requested_at"),
                    ci_green_at: None,
                    first_review_at: row.get::<Option<DateTime<Utc>>, _>("first_review_at"),
                    sent_back: row.get::<Option<i64>, _>("sent_back").unwrap_or(0),
                    claim_lapses: row.get::<Option<i64>, _>("claim_lapses").unwrap_or(0),
                    deliveries_skipped: 0,
                },
            )
        })
        .collect();
    let skipped = sqlx::query(
        "SELECT d.thread_id, COUNT(*)::BIGINT AS skipped
         FROM maidan_result_deliveries d
         WHERE d.status IN ('skipped', 'failed')
           AND d.thread_id IN (SELECT p.thread_id FROM maidan_change_pulls p WHERE p.workspace_id = $1)
         GROUP BY d.thread_id",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    for row in &skipped {
        flow.entry(row.get("thread_id"))
            .or_default()
            .deliveries_skipped = row.get("skipped");
    }
    // A head's checks as its CI notices said: a failure notice wins over a
    // finished one.
    let ci_rows = sqlx::query(
        "SELECT n.thread_id, n.head_sha, n.kind, n.status, n.created_at
         FROM maidan_thread_notices n
         WHERE n.workspace_id = $1 AND n.kind IN ('ci_failed', 'checks_done') AND n.head_sha IS NOT NULL",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    let mut head_ci: HashMap<(Uuid, String), maidan_types::CiStatus> = HashMap::new();
    for row in &ci_rows {
        let status = if row.get::<String, _>("kind") == "ci_failed" {
            Some(maidan_types::CiStatus::Failure)
        } else {
            row.get::<Option<String>, _>("status")
                .and_then(|s| match s.as_str() {
                    "success" => Some(maidan_types::CiStatus::Success),
                    "failure" => Some(maidan_types::CiStatus::Failure),
                    "pending" => Some(maidan_types::CiStatus::Pending),
                    _ => None,
                })
        };
        if status == Some(maidan_types::CiStatus::Success) {
            let at: DateTime<Utc> = row.get("created_at");
            let green = &mut flow.entry(row.get("thread_id")).or_default().ci_green_at;
            if green.is_none_or(|first| at < first) {
                *green = Some(at);
            }
        }
        if let Some(status) = status {
            let key = (
                row.get::<Uuid, _>("thread_id"),
                row.get::<String, _>("head_sha").to_ascii_lowercase(),
            );
            let entry = head_ci.entry(key).or_insert(status);
            if status == maidan_types::CiStatus::Failure {
                *entry = status;
            }
        }
    }
    for pull in &mut pulls {
        pull.flow = flow.get(&pull.thread_id.0).cloned().unwrap_or_default();
        pull.first_delivery_ci = pull.rounds.first().and_then(|round| {
            head_ci
                .get(&(pull.thread_id.0, round.commit_sha.to_ascii_lowercase()))
                .copied()
        });
    }
    Ok(pulls)
}

/// The merged pull requests in `workspace_id` due an outcome check, at most
/// `limit`: merged within the outcome window, not checked in the last day,
/// never-checked first. None while the workspace's last batch is under an
/// hour old, so the GitHub reads come in one batch an hour, not every tick.
pub async fn list_outcomes_due(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<OutcomeDue>, StoreError> {
    let rows = sqlx::query(
        "SELECT p.repo, p.pull_number, p.merged_at, p.linear_issue,
                p.merge_commit_sha, p.reverted_at, p.revert_ref, p.followup_fixes, p.human_commits,
                p.human_reviews, p.first_human_review_at,
                (SELECT r.base_branch FROM maidan_change_rounds r
                 WHERE r.workspace_id = p.workspace_id AND r.repo = p.repo
                   AND r.pull_number = p.pull_number
                 ORDER BY r.round DESC LIMIT 1) AS base_branch
         FROM maidan_change_pulls p
         WHERE p.workspace_id = $1 AND NOT p.watch_only AND p.merged_at IS NOT NULL AND p.merged_at >= $2
           AND (p.outcome_checked_at IS NULL OR p.outcome_checked_at <= $3)
           AND NOT EXISTS (
               SELECT 1 FROM maidan_change_pulls q
               WHERE q.workspace_id = $1 AND q.outcome_checked_at > $4)
         ORDER BY p.outcome_checked_at NULLS FIRST, p.merged_at, p.repo, p.pull_number
         LIMIT $5",
    )
    .bind(workspace_id.0)
    .bind(now - chrono::Duration::days(OUTCOME_WINDOW_DAYS))
    .bind(now - chrono::Duration::days(1))
    .bind(now - chrono::Duration::hours(1))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut due = Vec::with_capacity(rows.len());
    for row in &rows {
        let repo: String = row.get("repo");
        let pull_number: i64 = row.get("pull_number");
        let round_commits = sqlx::query_scalar(
            "SELECT commit_sha FROM maidan_change_rounds
             WHERE repo = $1 AND pull_number = $2 AND workspace_id = $3
             ORDER BY round",
        )
        .bind(&repo)
        .bind(pull_number)
        .bind(workspace_id.0)
        .fetch_all(pool)
        .await?;
        due.push(OutcomeDue {
            merged_at: row.get::<DateTime<Utc>, _>("merged_at"),
            base_branch: row.get("base_branch"),
            linear_issue: row.get("linear_issue"),
            outcome: outcome_from(row),
            round_commits,
            repo,
            pull_number,
        });
    }
    Ok(due)
}

/// Record what an outcome check found, whole, and stamp it checked.
pub async fn record_outcome(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
    outcome: &MergedOutcome,
    checked_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_change_pulls
         SET merge_commit_sha = $3, reverted_at = $4, revert_ref = $5, followup_fixes = $6,
             human_commits = $7, human_reviews = $8, first_human_review_at = $9,
             outcome_checked_at = $10
         WHERE repo = $1 AND pull_number = $2 AND workspace_id = $11",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(&outcome.merge_commit_sha)
    .bind(outcome.reverted_at)
    .bind(&outcome.revert_ref)
    .bind(Json(&outcome.followup_fixes))
    .bind(outcome.human_commits)
    .bind(outcome.human_reviews)
    .bind(outcome.first_human_review_at)
    .bind(checked_at)
    .bind(workspace_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

fn outcome_from(row: &sqlx::postgres::PgRow) -> MergedOutcome {
    MergedOutcome {
        merge_commit_sha: row.get("merge_commit_sha"),
        reverted_at: row.get::<Option<DateTime<Utc>>, _>("reverted_at"),
        revert_ref: row.get("revert_ref"),
        followup_fixes: row
            .try_get::<Json<Vec<i64>>, _>("followup_fixes")
            .map(|fixes| fixes.0)
            .unwrap_or_default(),
        human_commits: row.get("human_commits"),
        human_reviews: row.get("human_reviews"),
        first_human_review_at: row.get::<Option<DateTime<Utc>>, _>("first_human_review_at"),
    }
}

/// Hand a non-converging task to a session, once: record the handoff, end
/// its claim (its time charged as any claim end charges it, and its next
/// renew refused), route it to `session`, and append `ThreadHandoff`, in one
/// transaction. `None` when it was handed off already.
pub async fn hand_off(
    pool: &PgPool,
    thread_id: ThreadId,
    repo: &str,
    pull_number: i64,
    rounds: i64,
) -> Result<Option<maidan_types::HandedOff>, StoreError> {
    let mut tx = pool.begin().await?;
    let (workspace_id, channel_id) = events::thread_scope_in_tx(&mut tx, thread_id).await?;
    let now = Utc::now();
    let inserted = sqlx::query(
        "INSERT INTO maidan_thread_handoffs
             (thread_id, workspace_id, repo, pull_number, rounds, handed_off_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (thread_id) DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(workspace_id.0)
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(rounds)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(None);
    }
    // Read before the claim is cleared: the holder may be the only author.
    let author_id = sqlx::query(
        "SELECT COALESCE(t.owner_id, t.assignee_id, r.produced_by) AS author
         FROM maidan_threads t
         LEFT JOIN maidan_thread_results r ON r.thread_id = t.id
         WHERE t.id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .and_then(|row| row.get::<Option<Uuid>, _>("author"))
    .map(MemberId);
    super::threads::charge_open_claim_in_tx(&mut tx, thread_id).await?;
    sqlx::query(
        "UPDATE maidan_threads SET updated_at = $2,
            assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL,
            claimed_at = NULL, work_started_at = NULL
         WHERE id = $1",
    )
    .bind(thread_id.0)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO maidan_thread_attributes (thread_id, kind, engine, threat_model, updated_at)
         VALUES ($1, 'change', 'session', FALSE, $2)
         ON CONFLICT (thread_id) DO UPDATE SET engine = 'session', updated_at = excluded.updated_at",
    )
    .bind(thread_id.0)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let event = events::append_in_tx(
        &mut tx,
        &Event::ThreadHandoff {
            occurred_at: now,
            workspace_id,
            channel_id,
            thread_id,
            rounds,
            repo: repo.to_ascii_lowercase(),
            pr_number: pull_number,
            to_engine: maidan_types::TaskEngine::Session,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Some(maidan_types::HandedOff {
        thread_id,
        workspace_id,
        event,
        author_id,
    }))
}

/// Track a pull request the change flow did not open (a session's) on
/// `thread_id`. Its time to ready starts at the thread's `CLAIM:` notice,
/// else `claimed_at` (a claim said outside Maidan), else now. Registering it
/// again on the same thread answers the same; one another thread in the same
/// workspace tracks is a conflict. Another workspace tracking the same pull
/// request is not: each workspace keeps its own row, so none can claim a
/// pull request first and lock the others out of it.
pub async fn register(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
    repo: &str,
    pull_number: i64,
    claimed_at: Option<DateTime<Utc>>,
    watch_only: bool,
) -> Result<DateTime<Utc>, StoreError> {
    let repo = repo.to_ascii_lowercase();
    let mut tx = pool.begin().await?;
    let claimed: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT created_at FROM maidan_linear_notices WHERE thread_id = $1 AND kind = 'claim'",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?;
    let opened_at = claimed.or(claimed_at).unwrap_or_else(Utc::now);
    sqlx::query(
        "INSERT INTO maidan_change_pulls (repo, pull_number, workspace_id, thread_id, opened_at, watch_only)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (workspace_id, repo, pull_number) DO NOTHING",
    )
    .bind(&repo)
    .bind(pull_number)
    .bind(workspace_id.0)
    .bind(thread_id.0)
    .bind(opened_at)
    .bind(watch_only)
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query(
        "SELECT thread_id, opened_at FROM maidan_change_pulls
         WHERE repo = $1 AND pull_number = $2 AND workspace_id = $3",
    )
    .bind(&repo)
    .bind(pull_number)
    .bind(workspace_id.0)
    .fetch_one(&mut *tx)
    .await?;
    if row.get::<Uuid, _>("thread_id") != thread_id.0 {
        return Err(StoreError::Conflict(format!(
            "{repo}#{pull_number} is already tracked on another task"
        )));
    }
    let opened_at: DateTime<Utc> = row.get("opened_at");
    tx.commit().await?;
    Ok(opened_at)
}

fn pull_from(
    row: &sqlx::postgres::PgRow,
    rounds: &mut HashMap<(String, i64), Vec<ChangeRound>>,
    cost: &HashMap<ThreadId, maidan_types::ThreadSpend>,
) -> ChangePull {
    let repo: String = row.get("repo");
    let pull_number: i64 = row.get("pull_number");
    let thread: Uuid = row.get("thread_id");
    let spend = cost.get(&ThreadId(thread));
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
        cost_usd_micros: spend.map_or(0, |spend| spend.cost_usd_micros),
        provider: spend.and_then(|spend| spend.provider.clone()),
        harness: spend.and_then(|spend| spend.harness.clone()),
        linear_issue: row.get("linear_issue"),
        outcome: outcome_from(row),
        handed_off_at: row.get::<Option<DateTime<Utc>>, _>("handed_off_at"),
        flow: maidan_types::ThreadFlow::default(),
        first_delivery_ci: None,
        attributes: maidan_types::TaskAttributes {
            kind: row
                .try_get::<Option<String>, _>("task_kind")
                .ok()
                .flatten()
                .and_then(|kind| maidan_types::TaskKind::parse(&kind))
                .unwrap_or_default(),
            engine: row
                .try_get::<Option<String>, _>("task_engine")
                .ok()
                .flatten()
                .and_then(|engine| maidan_types::TaskEngine::parse(&engine)),
            threat_model: row
                .try_get::<Option<bool>, _>("threat_model")
                .ok()
                .flatten()
                .unwrap_or(false),
            size: row
                .try_get::<Option<String>, _>("task_size")
                .ok()
                .flatten()
                .and_then(|size| maidan_types::TaskSize::parse(&size)),
            acceptance: row
                .try_get::<Option<String>, _>("task_acceptance")
                .ok()
                .flatten(),
            linear_issue: row
                .try_get::<Option<String>, _>("task_linear_issue")
                .ok()
                .flatten(),
            tier: row
                .try_get::<Option<String>, _>("task_tier")
                .ok()
                .flatten()
                .and_then(|tier| maidan_types::TaskTier::parse(&tier)),
            lane: row
                .try_get::<Option<String>, _>("task_lane")
                .ok()
                .flatten()
                .and_then(|lane| maidan_types::TaskLane::parse(&lane)),
            review_family: None,
            author_family: None,
        },
    }
}
