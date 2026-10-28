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
//! The same read says what else holds the pull request back
//! ([`gate`]): unresolved review threads (a review bot's count), a merge
//! state such as `BEHIND` or `DIRTY`, and checks the base branch requires
//! that never reported. A change in those, once two reads agree on it, is
//! said as `gate_blocked` with the reasons, or `gate_clear`.
//!
//! It also says where the pull request stands with the merge queue
//! ([`merge_queue`]): `ready_to_merge` (the label the queue takes work
//! from), `queued`, `dequeued` with GitHub's reason, or `landed` once its
//! merge is recorded. Entering and leaving the queue are said as
//! `merge_queued` and `merge_dequeued`; a landing is the change sweep's
//! `ThreadLanded`, as for any merge.
//!
//! The one extra GitHub read is a failing job's log, and only when a head
//! first fails: the record keeps its last 4 KB, so a worker sees why without
//! opening GitHub, and later ticks over the same failing head reuse it.

use maidan_types::{
    CiStatus, MergeQueue, MergeQueueState, NewPullCi, WorkspaceId, FAILING_JOB_LIMIT,
    LOG_TAIL_BYTES, READY_LABEL, REVIEW_THREAD_LINK_LIMIT,
};

use crate::github::{GithubCheck, GithubCheckState, GithubGate, GithubGit, GithubPullSnapshot};
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

/// Merge states that hold a pull request back. `UNSTABLE` (a check that is
/// not required failed) and `UNKNOWN` (GitHub still computing) do not.
const BLOCKING_MERGE_STATES: &[&str] = &["BEHIND", "BLOCKED", "DIRTY", "DRAFT"];

/// What holds a pull request back, besides failing checks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Gate {
    /// Required checks that never reported though every check that did has
    /// finished: one that is not coming without a push or a rerun.
    pub missing_checks: Vec<String>,
    /// Each reason in a sentence, sorted, so two reads compare equal when
    /// nothing changed.
    pub reasons: Vec<String>,
}

/// What holds a pull request back, from GitHub's gate and the checks on its
/// head.
pub fn gate(read: &GithubGate, checks: &[GithubCheck]) -> Gate {
    let mut reasons = Vec::new();
    let unresolved = read.unresolved_threads.len();
    if unresolved > 0 {
        reasons.push(format!(
            "{unresolved} unresolved review thread{}",
            if unresolved == 1 { "" } else { "s" }
        ));
    }
    if let Some(state) = read
        .merge_state
        .as_deref()
        .filter(|state| BLOCKING_MERGE_STATES.contains(state))
    {
        reasons.push(format!("merge state {state}"));
    }
    let settled = !checks.is_empty() && checks.iter().all(|c| c.state != GithubCheckState::Pending);
    let missing_checks: Vec<String> = if settled {
        read.required_checks
            .iter()
            .flatten()
            .filter(|required| !checks.iter().any(|c| &c.name == *required))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    reasons.extend(
        missing_checks
            .iter()
            .map(|name| format!("required check `{name}` never reported")),
    );
    reasons.sort();
    Gate {
        missing_checks,
        reasons,
    }
}

/// Where an open pull request stands with the merge queue: in it, out of it
/// again (and why), or labelled for it; `None` when none of these.
pub fn merge_queue(read: &GithubGate, labels: &[String]) -> Option<MergeQueue> {
    if read.queued {
        return Some(MergeQueue {
            state: MergeQueueState::Queued,
            reason: None,
        });
    }
    if let Some(reason) = &read.dequeued {
        return Some(MergeQueue {
            state: MergeQueueState::Dequeued,
            reason: Some(reason.clone()),
        });
    }
    labels
        .iter()
        .any(|label| label == READY_LABEL)
        .then_some(MergeQueue {
            state: MergeQueueState::ReadyToMerge,
            reason: None,
        })
}

/// The repos whose checks GitHub refused, warned about once each.
fn forbidden_repos() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static REPOS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    REPOS.get_or_init(Default::default)
}

/// The text an operator sees when GitHub refuses `repo`'s checks.
fn checks_forbidden_text(repo: &str) -> String {
    format!(
        "GitHub refused the checks on {repo} (FORBIDDEN): the token lacks Checks: read and Commit statuses: read, so no checks_done or ci_failed notice can fire there"
    )
}

/// Say, once per repo, that GitHub refused its checks: a WARN in the log and
/// an operator alert (#1623).
async fn checks_forbidden(repo: &str) {
    let repo = repo.to_ascii_lowercase();
    let first = forbidden_repos()
        .lock()
        .map(|mut seen| seen.insert(repo.clone()))
        .unwrap_or(true);
    if !first {
        return;
    }
    let text = checks_forbidden_text(&repo);
    tracing::warn!(repo = %repo, "{text}");
    crate::ops_alert::alert(&format!("checks_forbidden:{repo}"), &text).await;
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
        if !snapshot.checks_forbidden {
            return 0;
        }
        checks_forbidden(repo).await;
        // The head still moved: record it, unreadable, so a push dismisses
        // the approvals given to the old head (#1623).
        return post_read(
            state,
            unreadable_read(workspace_id, repo, pull_number, snapshot),
        )
        .await;
    };
    let summary = summarize(checks);
    let head = snapshot.brief.head_sha.to_ascii_lowercase();
    let log_tail = if summary.failing_jobs.is_empty() {
        None
    } else {
        failing_log_tail(state, git, workspace_id, repo, pull_number, &head, &summary).await
    };
    let read_gate = snapshot.gate.as_ref();
    let held = read_gate.map(|read| gate(read, checks));
    let unresolved = read_gate
        .map(|read| read.unresolved_threads.clone())
        .unwrap_or_default();
    let read = NewPullCi {
        workspace_id,
        repo: repo.to_string(),
        pull_number,
        head_sha: head,
        status: summary.status,
        failing_jobs: summary.failing_jobs,
        log_tail,
        finished: summary.finished,
        merge_state: read_gate.and_then(|read| read.merge_state.clone()),
        unresolved_threads: unresolved.len() as i64,
        unresolved_thread_links: unresolved
            .into_iter()
            .take(REVIEW_THREAD_LINK_LIMIT)
            .collect(),
        missing_checks: held
            .as_ref()
            .map(|held| held.missing_checks.clone())
            .unwrap_or_default(),
        gate_reasons: held.map(|held| held.reasons),
        merge_queue: read_gate.and_then(|read| merge_queue(read, &snapshot.brief.labels)),
        head_ahead: snapshot.ahead.unwrap_or(false),
    };
    post_read(state, read).await
}

/// A read GitHub refused the checks of: the head and its merge state, with
/// the checks unreadable rather than pending or passing.
fn unreadable_read(
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
    snapshot: &GithubPullSnapshot,
) -> NewPullCi {
    let read_gate = snapshot.gate.as_ref();
    NewPullCi {
        workspace_id,
        repo: repo.to_string(),
        pull_number,
        head_sha: snapshot.brief.head_sha.to_ascii_lowercase(),
        status: CiStatus::Unreadable,
        failing_jobs: Vec::new(),
        log_tail: None,
        finished: false,
        merge_state: read_gate.and_then(|read| read.merge_state.clone()),
        unresolved_threads: 0,
        unresolved_thread_links: Vec::new(),
        missing_checks: Vec::new(),
        gate_reasons: None,
        merge_queue: read_gate.and_then(|read| merge_queue(read, &snapshot.brief.labels)),
        head_ahead: snapshot.ahead.unwrap_or(false),
    }
}

/// Record a read and post the notices it owes. Returns how many were posted.
async fn post_read(state: &AppState, read: NewPullCi) -> usize {
    let (repo, pull_number) = (read.repo.clone(), read.pull_number);
    let posted = match state.store.record_pull_ci(read).await {
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

    fn github_gate(merge_state: &str, threads: usize, required: &[&str]) -> GithubGate {
        GithubGate {
            merge_state: Some(merge_state.into()),
            unresolved_threads: (0..threads)
                .map(|n| maidan_types::ReviewThreadLink {
                    url: format!("https://example.test/{n}"),
                    author: None,
                })
                .collect(),
            required_checks: Some(required.iter().map(|r| r.to_string()).collect()),
            queued: false,
            dequeued: None,
        }
    }

    #[test]
    fn the_queue_outranks_a_dequeue_and_a_dequeue_outranks_the_label() {
        let ready = vec![READY_LABEL.to_string()];
        let mut read = github_gate("CLEAN", 0, &[]);
        assert_eq!(
            merge_queue(&read, &ready).map(|q| q.state),
            Some(MergeQueueState::ReadyToMerge)
        );
        assert_eq!(merge_queue(&read, &[]), None);
        read.dequeued = Some("MERGE_CONFLICT".into());
        assert_eq!(
            merge_queue(&read, &ready),
            Some(MergeQueue {
                state: MergeQueueState::Dequeued,
                reason: Some("MERGE_CONFLICT".into())
            })
        );
        read.queued = true;
        assert_eq!(
            merge_queue(&read, &ready).map(|q| q.state),
            Some(MergeQueueState::Queued)
        );
    }

    #[test]
    fn a_clean_pull_with_its_required_checks_in_is_not_held() {
        let checks = [check("lint", GithubCheckState::Passed, Some(1))];
        assert_eq!(
            gate(&github_gate("CLEAN", 0, &["lint"]), &checks),
            Gate::default()
        );
        assert_eq!(
            gate(&github_gate("UNSTABLE", 0, &[]), &checks),
            Gate::default(),
            "a failing check that is not required holds nothing"
        );
    }

    #[test]
    fn threads_merge_state_and_a_missing_required_check_each_hold_it() {
        let checks = [check("lint", GithubCheckState::Passed, Some(1))];
        let held = gate(&github_gate("BEHIND", 2, &["lint", "e2e"]), &checks);
        assert_eq!(held.missing_checks, vec!["e2e".to_string()]);
        assert_eq!(
            held.reasons,
            vec![
                "2 unresolved review threads".to_string(),
                "merge state BEHIND".to_string(),
                "required check `e2e` never reported".to_string(),
            ]
        );
        let running = [
            check("lint", GithubCheckState::Passed, Some(1)),
            check("unit", GithubCheckState::Pending, Some(2)),
        ];
        assert!(
            gate(&github_gate("CLEAN", 0, &["e2e"]), &running)
                .missing_checks
                .is_empty(),
            "a required check may still report while others run"
        );
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
