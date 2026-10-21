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
    /// GitHub's merge state for the pull request (`CLEAN`, `BEHIND`,
    /// `BLOCKED`, `DIRTY`, `DRAFT`, `UNSTABLE`, ...), when GitHub had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_state: Option<String>,
    /// How many review threads are unresolved, bots' (a review bot's) as
    /// well as people's.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unresolved_threads: i64,
    /// Where the first unresolved review threads are, at most 20.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_thread_links: Vec<ReviewThreadLink>,
    /// Checks the base branch's protection requires that never reported on
    /// this head, though every check that did report has finished.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_checks: Vec<String>,
    /// What holds the pull request back; empty when nothing the sweep reads
    /// does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate_reasons: Vec<String>,
    /// Where the pull request stands with its base branch's merge queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_queue: Option<MergeQueue>,
    /// When the sweep last read it.
    pub updated_at: DateTime<Utc>,
}

/// Where a pull request stands with the merge queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum MergeQueueState {
    /// It carries the `ready-to-merge` label the queue takes work from, and
    /// is not queued yet.
    ReadyToMerge,
    /// It is in the merge queue.
    Queued,
    /// It left the queue without merging; `reason` says why.
    Dequeued,
    /// It landed on its base: merged, or closed by a tool that landed its
    /// head itself.
    Landed,
}

impl MergeQueueState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadyToMerge => "ready_to_merge",
            Self::Queued => "queued",
            Self::Dequeued => "dequeued",
            Self::Landed => "landed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ready_to_merge" => Some(Self::ReadyToMerge),
            "queued" => Some(Self::Queued),
            "dequeued" => Some(Self::Dequeued),
            "landed" => Some(Self::Landed),
            _ => None,
        }
    }
}

/// A pull request's merge-queue state, and why it was dequeued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeQueue {
    pub state: MergeQueueState,
    /// GitHub's reason for a dequeue (`MERGE_CONFLICT`, `FAILED_CHECKS`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What the last read recorded, which a new read is compared with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviousRead {
    /// The reasons that held the pull request back; `None` before any read
    /// could say.
    pub gate: Option<Vec<String>>,
    /// Its merge-queue state; `None` when it had none.
    pub merge_queue: Option<MergeQueue>,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Where an unresolved review thread is, and who opened it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewThreadLink {
    pub url: String,
    /// The login of the first comment's author, a bot's included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// Review-thread links a record keeps, at most.
pub const REVIEW_THREAD_LINK_LIMIT: usize = 20;

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
    pub merge_state: Option<String>,
    pub unresolved_threads: i64,
    pub unresolved_thread_links: Vec<ReviewThreadLink>,
    pub missing_checks: Vec<String>,
    /// What holds the pull request back, sorted; `None` when the read could
    /// not say, which leaves the last answer standing.
    pub gate_reasons: Option<Vec<String>>,
    /// Its merge-queue state, read with the gate: compared only when
    /// `gate_reasons` was read.
    pub merge_queue: Option<MergeQueue>,
    /// Whether the head has commits its base lacks. Kept once seen for the
    /// same head, so a head the base caught up with still counts as ahead.
    pub head_ahead: bool,
}

/// The notices a read owes its thread, each with the key that says it once:
/// `pr_opened` once per pull request (the first head it was seen at is the
/// one it names), `ci_failed` and `checks_done` once per head commit.
///
/// `gate_blocked` and `gate_clear` are said when what holds the pull request
/// back changed since `previous_gate` (the last read's reasons; `None` before
/// the first read): blocked with the new reasons, clear when there are none
/// left. Their keys carry `change`, unique to this change, because the same
/// reasons can come back after a clear; the caller compares under the lock
/// that serializes reads, so a change is said once.
///
/// `merge_queued` and `merge_dequeued` are said, the same way, when the
/// pull request enters the merge queue or leaves it (or leaves it again for
/// another reason).
pub fn ci_notices(
    new: &NewPullCi,
    previous: &PreviousRead,
    change: &str,
    now: DateTime<Utc>,
) -> Vec<(ThreadNotice, String)> {
    let previous_gate = previous.gate.as_deref();
    let repo = new.repo.to_ascii_lowercase();
    let head = new.head_sha.to_ascii_lowercase();
    let at = format!("{repo}#{}", new.pull_number);
    let notice = |kind, status, failing_jobs, reasons| ThreadNotice {
        kind,
        repo: repo.clone(),
        pull_number: new.pull_number,
        paths: Vec::new(),
        base_branch: None,
        base_behind: None,
        head_sha: Some(head.clone()),
        status,
        failing_jobs,
        reasons,
        created_at: now,
    };
    let mut owed = vec![(
        notice(ThreadNoticeKind::PrOpened, None, Vec::new(), Vec::new()),
        format!("pr_opened:{at}"),
    )];
    if !new.failing_jobs.is_empty() {
        owed.push((
            notice(
                ThreadNoticeKind::CiFailed,
                Some(CiStatus::Failure),
                new.failing_jobs.clone(),
                Vec::new(),
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
                Vec::new(),
            ),
            format!("checks_done:{at}@{head}"),
        ));
    }
    if let Some(reasons) = &new.gate_reasons {
        let before = previous_gate.unwrap_or_default();
        if reasons.as_slice() != before {
            if !reasons.is_empty() {
                owed.push((
                    notice(
                        ThreadNoticeKind::GateBlocked,
                        None,
                        Vec::new(),
                        reasons.clone(),
                    ),
                    format!("gate:{at}:{change}"),
                ));
            } else {
                owed.push((
                    notice(ThreadNoticeKind::GateClear, None, Vec::new(), Vec::new()),
                    format!("gate:{at}:{change}"),
                ));
            }
        }
    }
    if new.gate_reasons.is_some() && new.merge_queue != previous.merge_queue {
        let entered = new.merge_queue.as_ref().map(|queue| queue.state);
        match (entered, new.merge_queue.as_ref()) {
            (Some(MergeQueueState::Queued), _) => owed.push((
                notice(ThreadNoticeKind::MergeQueued, None, Vec::new(), Vec::new()),
                format!("queue:{at}:{change}"),
            )),
            (Some(MergeQueueState::Dequeued), Some(queue)) => owed.push((
                notice(
                    ThreadNoticeKind::MergeDequeued,
                    None,
                    Vec::new(),
                    queue.reason.iter().cloned().collect(),
                ),
                format!("queue:{at}:{change}"),
            )),
            _ => {}
        }
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
            merge_state: None,
            unresolved_threads: 0,
            unresolved_thread_links: Vec::new(),
            missing_checks: Vec::new(),
            gate_reasons: None,
            merge_queue: None,
            head_ahead: false,
        }
    }

    fn ci_notices_at(new: &NewPullCi, now: DateTime<Utc>) -> Vec<(ThreadNotice, String)> {
        ci_notices(new, &PreviousRead::default(), "c1", now)
    }

    fn prev(gate: Option<&[String]>) -> PreviousRead {
        PreviousRead {
            gate: gate.map(<[String]>::to_vec),
            merge_queue: None,
        }
    }

    #[test]
    fn entering_and_leaving_the_queue_is_said_on_the_change() {
        let mut new = read(CiStatus::Pending, &[], false);
        new.gate_reasons = Some(Vec::new());
        let queue_kinds = |owed: Vec<(ThreadNotice, String)>| {
            owed.into_iter()
                .filter(|(n, _)| n.kind.as_str().starts_with("merge_"))
                .map(|(n, _)| (n.kind.as_str(), n.reasons))
                .collect::<Vec<_>>()
        };
        let queued = MergeQueue {
            state: MergeQueueState::Queued,
            reason: None,
        };
        new.merge_queue = Some(queued.clone());
        let before = PreviousRead {
            gate: Some(Vec::new()),
            merge_queue: None,
        };
        assert_eq!(
            queue_kinds(ci_notices(&new, &before, "q1", Utc::now())),
            vec![("merge_queued", Vec::new())]
        );
        let in_queue = PreviousRead {
            merge_queue: Some(queued),
            ..before.clone()
        };
        assert!(queue_kinds(ci_notices(&new, &in_queue, "q2", Utc::now())).is_empty());
        new.merge_queue = Some(MergeQueue {
            state: MergeQueueState::Dequeued,
            reason: Some("MERGE_CONFLICT".into()),
        });
        assert_eq!(
            queue_kinds(ci_notices(&new, &in_queue, "q3", Utc::now())),
            vec![("merge_dequeued", vec!["MERGE_CONFLICT".to_string()])]
        );
        new.gate_reasons = None;
        assert!(
            queue_kinds(ci_notices(&new, &in_queue, "q4", Utc::now())).is_empty(),
            "a read that carried no gate says nothing about the queue"
        );
    }

    #[test]
    fn a_gate_is_said_only_when_what_holds_it_changes() {
        let mut new = read(CiStatus::Pending, &[], false);
        let gate = |owed: &[(ThreadNotice, String)]| {
            owed.iter()
                .filter(|(n, _)| {
                    matches!(
                        n.kind,
                        ThreadNoticeKind::GateBlocked | ThreadNoticeKind::GateClear
                    )
                })
                .map(|(n, key)| (n.kind.as_str(), n.reasons.clone(), key.clone()))
                .collect::<Vec<_>>()
        };
        new.gate_reasons = Some(Vec::new());
        assert!(
            gate(&ci_notices(&new, &prev(None), "c1", Utc::now())).is_empty(),
            "clear from the start is no news"
        );
        new.gate_reasons = Some(vec!["merge state BEHIND".into()]);
        assert_eq!(
            gate(&ci_notices(&new, &prev(Some(&[])), "c2", Utc::now())),
            vec![(
                "gate_blocked",
                vec!["merge state BEHIND".to_string()],
                "gate:example/app#7:c2".to_string()
            )]
        );
        let same = ["merge state BEHIND".to_string()];
        assert!(gate(&ci_notices(&new, &prev(Some(&same)), "c3", Utc::now())).is_empty());
        new.gate_reasons = Some(Vec::new());
        assert_eq!(
            gate(&ci_notices(&new, &prev(Some(&same)), "c4", Utc::now()))[0].0,
            "gate_clear"
        );
        new.gate_reasons = None;
        assert!(
            gate(&ci_notices(&new, &prev(Some(&same)), "c5", Utc::now())).is_empty(),
            "an unread gate says nothing"
        );
    }

    fn kinds(owed: &[(ThreadNotice, String)]) -> Vec<(&'static str, &str)> {
        owed.iter()
            .map(|(notice, key)| (notice.kind.as_str(), key.as_str()))
            .collect()
    }

    #[test]
    fn a_pending_head_owes_only_the_opened_notice() {
        let owed = ci_notices_at(&read(CiStatus::Pending, &[], false), Utc::now());
        assert_eq!(kinds(&owed), vec![("pr_opened", "pr_opened:example/app#7")]);
    }

    #[test]
    fn a_failure_while_others_run_is_said_before_the_checks_are_done() {
        let running = ci_notices_at(&read(CiStatus::Failure, &["unit tests"], false), Utc::now());
        assert_eq!(
            kinds(&running),
            vec![
                ("pr_opened", "pr_opened:example/app#7"),
                ("ci_failed", "ci_failed:example/app#7@abc123"),
            ]
        );
        let owed = ci_notices_at(&read(CiStatus::Failure, &["unit tests"], true), Utc::now());
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
        let owed = ci_notices_at(&read(CiStatus::Success, &[], true), Utc::now());
        let done = &owed[1].0;
        assert_eq!(done.kind, ThreadNoticeKind::ChecksDone);
        assert_eq!(done.status, Some(CiStatus::Success));
        assert_eq!(
            done.sentence(),
            "Checks done on example/app#7 at abc123: success."
        );
    }
}
