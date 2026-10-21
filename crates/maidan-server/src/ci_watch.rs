//! CI on tracked pull requests.
//!
//! The change sweep reads each open tracked pull request once a tick
//! ([`crate::github::GithubGit::pull_snapshot`], one GraphQL query), and the
//! checks on its head come with that read. [`record_checks`] turns them into
//! the thread's CI record (`ci` on `GET /threads/{id}` and in the context
//! pack) and the notices it owes: `pr_opened`, `ci_failed` and
//! `checks_done`, each said once per head commit
//! ([`maidan_types::ci_notices`]).
//!
//! The one extra GitHub read is a failing job's log, and only when a head
//! first fails: the record keeps its last 4 KB, so a worker sees why without
//! opening GitHub, and later ticks over the same failing head reuse it.

use maidan_types::{CiStatus, NewPullCi, WorkspaceId, FAILING_JOB_LIMIT, LOG_TAIL_BYTES};

use crate::github::{GithubCheck, GithubCheckState, GithubGit, GithubPullSnapshot};
use crate::state::AppState;

/// Where a head's checks stand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiSummary {
    pub status: CiStatus,
    /// The failed checks, by name, in GitHub's order, at most 20.
    pub failing_jobs: Vec<String>,
    /// Whether every check has finished. Nothing reported yet is not
    /// finished: the checks may not have started.
    pub finished: bool,
    /// The first failed check GitHub Actions ran: its job's log is the one
    /// the record keeps the end of.
    pub first_failing_job: Option<i64>,
}

/// Summarize the checks on a head. Any failure makes it a failure at once,
/// whatever still runs, as GitHub's own rollup does; otherwise it is pending
/// until every check has finished.
pub fn summarize(checks: &[GithubCheck]) -> CiSummary {
    let mut failing_jobs: Vec<String> = Vec::new();
    let mut first_failing_job = None;
    for check in checks
        .iter()
        .filter(|c| c.state == GithubCheckState::Failed)
    {
        if first_failing_job.is_none() {
            first_failing_job = check.job_id;
        }
        if failing_jobs.len() < FAILING_JOB_LIMIT && !failing_jobs.contains(&check.name) {
            failing_jobs.push(check.name.clone());
        }
    }
    let running = checks.iter().any(|c| c.state == GithubCheckState::Pending);
    let finished = !checks.is_empty() && !running;
    let status = if !failing_jobs.is_empty() {
        CiStatus::Failure
    } else if finished {
        CiStatus::Success
    } else {
        CiStatus::Pending
    };
    CiSummary {
        status,
        failing_jobs,
        finished,
        first_failing_job,
    }
}

/// Record the checks an open pull request's snapshot carried, and post the
/// notices they owe its thread. Returns how many were posted. Best effort: a
/// failed write is logged and the next tick reads again.
pub async fn record_checks(
    state: &AppState,
    git: &dyn GithubGit,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
    snapshot: &GithubPullSnapshot,
) -> usize {
    let Some(checks) = snapshot.checks.as_deref() else {
        return 0;
    };
    let summary = summarize(checks);
    let head = snapshot.brief.head_sha.to_ascii_lowercase();
    let log_tail = if summary.failing_jobs.is_empty() {
        None
    } else {
        failing_log_tail(state, git, workspace_id, repo, pull_number, &head, &summary).await
    };
    let posted = match state
        .store
        .record_pull_ci(NewPullCi {
            workspace_id,
            repo: repo.to_string(),
            pull_number,
            head_sha: head,
            status: summary.status,
            failing_jobs: summary.failing_jobs,
            log_tail,
            finished: summary.finished,
        })
        .await
    {
        Ok(posted) => posted,
        Err(err) => {
            tracing::warn!(repo, pull_number, error = %err, "ci watch: checks not recorded");
            return 0;
        }
    };
    let count = posted.len();
    for notice in posted {
        crate::change_sweep::post_notice(state, notice).await;
    }
    count
}

/// The failing job's log tail for a failing head: the one already recorded
/// when this head was failing at the last read, else read now, once.
async fn failing_log_tail(
    state: &AppState,
    git: &dyn GithubGit,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
    head: &str,
    summary: &CiSummary,
) -> Option<String> {
    let previous = match state
        .store
        .get_pull_ci(workspace_id, repo, pull_number)
        .await
    {
        Ok(previous) => previous,
        Err(err) => {
            tracing::warn!(repo, pull_number, error = %err, "ci watch: last read not loaded");
            None
        }
    };
    if let Some(previous) = previous.filter(|p| p.head_sha == head && !p.failing_jobs.is_empty()) {
        return previous.log_tail;
    }
    let job = summary.first_failing_job?;
    match git.job_log_tail(repo, job, LOG_TAIL_BYTES).await {
        Ok(tail) => Some(tail).filter(|tail| !tail.trim().is_empty()),
        Err(err) => {
            tracing::debug!(repo, pull_number, job, error = %err, "ci watch: job log not read");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &str, state: GithubCheckState, job_id: Option<i64>) -> GithubCheck {
        GithubCheck {
            name: name.into(),
            state,
            job_id,
        }
    }

    #[test]
    fn nothing_reported_yet_is_pending_and_not_finished() {
        let summary = summarize(&[]);
        assert_eq!(summary.status, CiStatus::Pending);
        assert!(!summary.finished);
    }

    #[test]
    fn a_failure_counts_at_once_while_other_checks_still_run() {
        let summary = summarize(&[
            check("lint", GithubCheckState::Passed, Some(1)),
            check("ci/status", GithubCheckState::Failed, None),
            check("unit tests", GithubCheckState::Failed, Some(3)),
            check("smoke", GithubCheckState::Pending, Some(4)),
        ]);
        assert_eq!(summary.status, CiStatus::Failure);
        assert_eq!(summary.failing_jobs, vec!["ci/status", "unit tests"]);
        assert!(!summary.finished);
        assert_eq!(
            summary.first_failing_job,
            Some(3),
            "the first failure is a status, which has no log to read"
        );
    }

    #[test]
    fn every_check_passed_is_a_finished_success() {
        let summary = summarize(&[
            check("lint", GithubCheckState::Passed, Some(1)),
            check("unit tests", GithubCheckState::Passed, Some(2)),
        ]);
        assert_eq!(summary.status, CiStatus::Success);
        assert!(summary.finished);
        assert!(summary.failing_jobs.is_empty());
    }
}
