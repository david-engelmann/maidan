//! CI on tracked pull requests.
//!
//! Each change-sweep tick reads every open tracked pull request from GitHub
//! once, and the checks on its head come with that read. The sweep records
//! where they stand on the pull request's thread ([`PullCi`], on
//! `GET /threads/{id}` and in the context pack) and owes the thread a few
//! notices ([`ci_notices`]): `pr_opened` the first time it sees the pull
//! request open, `ci_failed` when a check on a head fails, and
//! `checks_done` when every check on a head has finished. Each is said once
//! per head commit, so a push that starts the checks again is news and the
//! next tick over the same head is not.
//!
//! A pull request is tracked when the change flow opened it, when a session
//! registered it on its task (`PUT /threads/{id}/pull`), or when someone
//! asked to watch it ([`WatchPull`]), which gives it a thread of its own when
//! it has none.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::change_metrics::{ThreadNotice, ThreadNoticeKind};
use crate::ids::{ChannelId, ThreadId, WorkspaceId};

/// Failing checks a record or notice names, at most.
pub const FAILING_JOB_LIMIT: usize = 20;

/// The most of a failing job's log a record keeps: its last 4 KB, where the
/// error usually is.
pub const LOG_TAIL_BYTES: usize = 4096;

/// The channel a watched pull request's thread goes in when the caller names
/// none. Created, public, the first time it is needed.
pub const DEFAULT_WATCH_CHANNEL: &str = "pull-requests";

/// How the checks on a pull request's head stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum CiStatus {
    /// A check is still running or queued, none has failed, or none has
    /// reported yet.
    Pending,
    /// Every check finished and none failed.
    Success,
    /// A check failed, whether or not the others have finished.
    Failure,
}

impl CiStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "success" => Some(Self::Success),
            "failure" => Some(Self::Failure),
            _ => None,
        }
    }
}

/// Where the checks on a tracked pull request stand, as the change sweep last
/// read them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PullCi {
    /// Lowercase `owner/name`.
    pub repo: String,
    pub pull_number: i64,
    /// The head commit the checks ran on.
    pub head_sha: String,
    pub status: CiStatus,
    /// The checks that failed on this head, at most 20.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failing_jobs: Vec<String>,
    /// The last 4 KB of the first failing job's log, when GitHub Actions ran
    /// it. Read once, when the head first fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_tail: Option<String>,
    /// When the sweep last read it.
    pub updated_at: DateTime<Utc>,
}

/// A fresh read of a tracked pull request's checks, to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPullCi {
    pub workspace_id: WorkspaceId,
    /// Lowercase `owner/name`.
    pub repo: String,
    pub pull_number: i64,
    pub head_sha: String,
    pub status: CiStatus,
    pub failing_jobs: Vec<String>,
    pub log_tail: Option<String>,
    /// Whether every check on the head has finished. A failure is said as
    /// soon as it happens; the checks are done only once nothing still runs.
    pub finished: bool,
}

/// The notices a read owes its thread, each with the key that says it once:
/// `pr_opened` once per pull request (the first head it was seen at is the
/// one it names), `ci_failed` and `checks_done` once per head commit.
pub fn ci_notices(new: &NewPullCi, now: DateTime<Utc>) -> Vec<(ThreadNotice, String)> {
    let repo = new.repo.to_ascii_lowercase();
    let head = new.head_sha.to_ascii_lowercase();
    let at = format!("{repo}#{}", new.pull_number);
    let notice = |kind, status, failing_jobs| ThreadNotice {
        kind,
        repo: repo.clone(),
        pull_number: new.pull_number,
        paths: Vec::new(),
        base_branch: None,
        base_behind: None,
        head_sha: Some(head.clone()),
        status,
        failing_jobs,
        created_at: now,
    };
    let mut owed = vec![(
        notice(ThreadNoticeKind::PrOpened, None, Vec::new()),
        format!("pr_opened:{at}"),
    )];
    if !new.failing_jobs.is_empty() {
        owed.push((
            notice(
                ThreadNoticeKind::CiFailed,
                Some(CiStatus::Failure),
                new.failing_jobs.clone(),
            ),
            format!("ci_failed:{at}@{head}"),
        ));
    }
    if new.finished && new.status != CiStatus::Pending {
        owed.push((
            notice(
                ThreadNoticeKind::ChecksDone,
                Some(new.status),
                new.failing_jobs.clone(),
            ),
            format!("checks_done:{at}@{head}"),
        ));
    }
    owed
}

/// Watch a pull request: track it, so each change-sweep tick reads its
/// checks, and give it a thread when it has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WatchPull {
    /// `owner/name`; it must be in the workspace's GitHub egress allowlist.
    pub repo: String,
    pub pull_number: i64,
    /// Where a new watch thread goes; the workspace's `pull-requests`
    /// channel (created when missing) when omitted. Unused when the pull
    /// request already has a thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<ChannelId>,
    /// A new watch thread's title; `owner/name#N` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// A watched pull request and the thread its notices go to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WatchedPull {
    /// Lowercase `owner/name`.
    pub repo: String,
    pub pull_number: i64,
    pub thread_id: ThreadId,
    /// Whether this call made the thread; `false` when the pull request was
    /// already tracked on one.
    pub created: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(status: CiStatus, failing: &[&str], finished: bool) -> NewPullCi {
        NewPullCi {
            workspace_id: WorkspaceId(uuid::Uuid::nil()),
            repo: "Example/App".into(),
            pull_number: 7,
            head_sha: "ABC123".into(),
            status,
            failing_jobs: failing.iter().map(|job| job.to_string()).collect(),
            log_tail: None,
            finished,
        }
    }

    fn kinds(owed: &[(ThreadNotice, String)]) -> Vec<(&'static str, &str)> {
        owed.iter()
            .map(|(notice, key)| (notice.kind.as_str(), key.as_str()))
            .collect()
    }

    #[test]
    fn a_pending_head_owes_only_the_opened_notice() {
        let owed = ci_notices(&read(CiStatus::Pending, &[], false), Utc::now());
        assert_eq!(kinds(&owed), vec![("pr_opened", "pr_opened:example/app#7")]);
    }

    #[test]
    fn a_failure_while_others_run_is_said_before_the_checks_are_done() {
        let running = ci_notices(&read(CiStatus::Failure, &["unit tests"], false), Utc::now());
        assert_eq!(
            kinds(&running),
            vec![
                ("pr_opened", "pr_opened:example/app#7"),
                ("ci_failed", "ci_failed:example/app#7@abc123"),
            ]
        );
        let owed = ci_notices(&read(CiStatus::Failure, &["unit tests"], true), Utc::now());
        assert_eq!(
            kinds(&owed),
            vec![
                ("pr_opened", "pr_opened:example/app#7"),
                ("ci_failed", "ci_failed:example/app#7@abc123"),
                ("checks_done", "checks_done:example/app#7@abc123"),
            ]
        );
        assert_eq!(owed[1].0.failing_jobs, vec!["unit tests".to_string()]);
        assert_eq!(owed[2].0.status, Some(CiStatus::Failure));
    }

    #[test]
    fn a_green_head_says_its_checks_are_done_with_success() {
        let owed = ci_notices(&read(CiStatus::Success, &[], true), Utc::now());
        let done = &owed[1].0;
        assert_eq!(done.kind, ThreadNoticeKind::ChecksDone);
        assert_eq!(done.status, Some(CiStatus::Success));
        assert_eq!(
            done.sentence(),
            "Checks done on example/app#7 at abc123: success."
        );
    }
}
