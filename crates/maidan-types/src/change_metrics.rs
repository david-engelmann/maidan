//! Swarm ramp metrics: what each agent pull request cost to get merged.
//!
//! The change flow records every round it delivers to an agent pull request
//! ([`NewChangeRound`]): the commit, how large its diff was, and how far its
//! base had fallen behind the base branch. Mark-ready stamps when the pull
//! request was made ready, and the change sweep refreshes whether it merged
//! (and settles its thread: [`ChangeSettlement`]). The
//! metrics a swarm's ramp is gated on (rounds per task, time to ready,
//! merged without anyone else's commits, cost per merged change) and two
//! warnings (a task that is not converging, a round built on a stale base)
//! are computed from those rows ([`SwarmMetrics`]).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::events::StoredEvent;
use crate::ids::{MemberId, ThreadId, WorkspaceId};

/// What settling a closed agent pull request did to its thread. Each close
/// is settled once.
#[derive(Debug)]
pub enum ChangeSettlement {
    /// It merged and the thread closed. The transitions' `ThreadStateChanged`
    /// events and the `ThreadLanded`, committed with the close, to publish.
    Landed(Vec<StoredEvent>),
    /// It closed without merging. The thread stays open for a person or the
    /// planner to decide, and `author_id` posts the one notice.
    ClosedUnmerged {
        thread_id: ThreadId,
        author_id: MemberId,
    },
    /// It merged, but the thread's close was refused (a review requirement
    /// not met, say). The thread stays open and `reason` is posted once.
    CloseRefused {
        thread_id: ThreadId,
        author_id: MemberId,
        reason: String,
    },
    /// Nothing to do: settled already, the thread is closed or gone, or
    /// another of its pull requests is still open.
    Unchanged,
}

/// One round the change flow delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewChangeRound {
    pub workspace_id: WorkspaceId,
    pub thread_id: ThreadId,
    /// Lowercase `owner/name`.
    pub repo: String,
    pub pull_number: i64,
    pub commit_sha: String,
    pub base_sha: String,
    /// Commits on the base branch since `base_sha` when the round landed;
    /// `None` when GitHub could not say.
    pub base_behind: Option<i64>,
    /// Added plus removed lines in the round's diff.
    pub diff_lines: i64,
    /// The files the round's diff touched ([`diff_paths`]).
    pub paths: Vec<String>,
    /// The branch the pull request merges into.
    pub base_branch: Option<String>,
    /// A squash-rebase round: the branch was rebuilt on the base's head.
    pub rebase: bool,
    /// For a squash-rebase round, the head the branch was force-moved from.
    pub previous_head: Option<String>,
    /// The Linear issue the change is for, when the result names one or it
    /// was delivered to one. The first round that knows it records it on
    /// the pull request.
    pub linear_issue: Option<String>,
}

/// What a change notice tells a thread's worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ThreadNoticeKind {
    /// A merged pull request touched paths the thread's claim covers.
    RebaseNeeded,
    /// A round delivered for the thread was built on a base at least
    /// `MAIDAN_STALE_BASE_COMMITS` behind its branch.
    StaleBase,
    /// The change sweep saw the thread's pull request open for the first
    /// time.
    PrOpened,
    /// A check on the pull request's head failed.
    CiFailed,
    /// Every check on the pull request's head finished; `status` says how.
    ChecksDone,
    /// The pull request cannot merge yet, or not for a new reason: unresolved
    /// review threads, a merge state such as `BEHIND` or `DIRTY`, or a
    /// required check that never reported. `reasons` says which.
    GateBlocked,
    /// Nothing the sweep reads holds the pull request back any more.
    GateClear,
    /// The pull request entered its base branch's merge queue.
    MergeQueued,
    /// The pull request left the merge queue without merging; `reasons`
    /// holds GitHub's reason (`MERGE_CONFLICT`, `FAILED_CHECKS`, ...).
    MergeDequeued,
    /// A deploy hook reported the pull request's merge commit deployed to
    /// `environment`; `status` says whether it succeeded.
    Deployed,
}

impl ThreadNoticeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RebaseNeeded => "rebase_needed",
            Self::StaleBase => "stale_base",
            Self::PrOpened => "pr_opened",
            Self::CiFailed => "ci_failed",
            Self::ChecksDone => "checks_done",
            Self::GateBlocked => "gate_blocked",
            Self::GateClear => "gate_clear",
            Self::MergeQueued => "merge_queued",
            Self::MergeDequeued => "merge_dequeued",
            Self::Deployed => "deployed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rebase_needed" => Some(Self::RebaseNeeded),
            "stale_base" => Some(Self::StaleBase),
            "pr_opened" => Some(Self::PrOpened),
            "ci_failed" => Some(Self::CiFailed),
            "checks_done" => Some(Self::ChecksDone),
            "gate_blocked" => Some(Self::GateBlocked),
            "gate_clear" => Some(Self::GateClear),
            "merge_queued" => Some(Self::MergeQueued),
            "merge_dequeued" => Some(Self::MergeDequeued),
            "deployed" => Some(Self::Deployed),
            _ => None,
        }
    }
}

/// Overlapping paths a `rebase_needed` notice names, at most.
pub const NOTICE_PATH_LIMIT: usize = 20;
/// Paths a round records, at most.
pub const ROUND_PATH_LIMIT: usize = 500;
/// Notices a thread read shows, newest first.
pub const THREAD_NOTICE_LIMIT: i64 = 10;

/// A change notice on a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadNotice {
    pub kind: ThreadNoticeKind,
    /// Lowercase `owner/name`.
    pub repo: String,
    /// The merged pull request (`rebase_needed`), the one the stale round
    /// went to (`stale_base`), or the one the checks ran on.
    pub pull_number: i64,
    /// The merged paths the claim covers, at most 20 ([`NOTICE_PATH_LIMIT`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_behind: Option<i64>,
    /// The head commit a CI notice is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    /// How the checks stand (`checks_done`, `ci_failed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<crate::pull_ci::CiStatus>,
    /// The checks that failed, at most [`crate::pull_ci::FAILING_JOB_LIMIT`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failing_jobs: Vec<String>,
    /// What holds the pull request back (`gate_blocked`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    /// Where a `deployed` notice's commit went (`production`, `staging`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// The deploy's own page, as the hook sent it (`deployed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ThreadNotice {
    /// The notice as one sentence, for the message posted with it.
    pub fn sentence(&self) -> String {
        let base = self.base_branch.as_deref().unwrap_or("its base branch");
        match self.kind {
            ThreadNoticeKind::RebaseNeeded => format!(
                "Rebase needed: {}#{} merged into {base} and touched {} that your claim covers: {}.",
                self.repo,
                self.pull_number,
                if self.paths.len() == 1 { "a path" } else { "paths" },
                self.paths.join(", ")
            ),
            ThreadNoticeKind::StaleBase => format!(
                "Stale base: the round delivered to {}#{} was built {} commits behind {base}; rebase before the next delivery.",
                self.repo,
                self.pull_number,
                self.base_behind.unwrap_or_default()
            ),
            ThreadNoticeKind::PrOpened => format!(
                "Pull request opened: {}#{}{}.",
                self.repo,
                self.pull_number,
                self.at_head()
            ),
            ThreadNoticeKind::CiFailed => format!(
                "CI failed on {}#{}{}: {}.",
                self.repo,
                self.pull_number,
                self.at_head(),
                self.failing_jobs.join(", ")
            ),
            ThreadNoticeKind::ChecksDone => format!(
                "Checks done on {}#{}{}: {}.",
                self.repo,
                self.pull_number,
                self.at_head(),
                self.status.map_or("unknown", |status| status.as_str())
            ),
            ThreadNoticeKind::GateBlocked => format!(
                "{}#{} cannot merge yet: {}.",
                self.repo,
                self.pull_number,
                self.reasons.join("; ")
            ),
            ThreadNoticeKind::MergeQueued => format!(
                "{}#{} entered the merge queue.",
                self.repo, self.pull_number
            ),
            ThreadNoticeKind::MergeDequeued => format!(
                "{}#{} left the merge queue without merging: {}.",
                self.repo,
                self.pull_number,
                self.reasons.join(", ")
            ),
            ThreadNoticeKind::GateClear => format!(
                "{}#{} is no longer held back.",
                self.repo, self.pull_number
            ),
            ThreadNoticeKind::Deployed => {
                let environment = self.environment.as_deref().unwrap_or("an environment");
                let link = self
                    .deploy_url
                    .as_deref()
                    .map(|url| format!(" ({url})"))
                    .unwrap_or_default();
                if self.status == Some(crate::pull_ci::CiStatus::Failure) {
                    format!(
                        "Deploy to {environment} failed for {}#{}{}{link}.",
                        self.repo,
                        self.pull_number,
                        self.at_head()
                    )
                } else {
                    format!(
                        "{}#{} is deployed to {environment}{}{link}.",
                        self.repo,
                        self.pull_number,
                        self.at_head()
                    )
                }
            }
        }
    }

    /// ` at <short sha>`, or nothing when the notice names no head.
    fn at_head(&self) -> String {
        self.head_sha
            .as_deref()
            .map(|sha| format!(" at {}", sha.chars().take(12).collect::<String>()))
            .unwrap_or_default()
    }
}

/// A notice the store recorded, with its event to publish and who posts
/// the message that says it (the thread's owner, else its claim holder, else
/// whoever produced its result; `None` when the thread has none of them).
#[derive(Debug, Clone)]
pub struct PostedNotice {
    pub thread_id: ThreadId,
    pub notice: ThreadNotice,
    pub event: StoredEvent,
    pub author_id: Option<MemberId>,
}

/// A deploy a deploy hook reported: `sha` reached `environment`, or failed
/// to. Matched to the merged pull requests whose merge (or merged head)
/// commit it names, and to those in `contains` the server found `sha`
/// carries as an ancestor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDeploy {
    /// Lowercase `owner/name`.
    pub repo: String,
    /// The full commit SHA, lowercase.
    pub sha: String,
    pub environment: String,
    /// `success` or `failure`; a deploy still running says nothing yet.
    pub status: crate::pull_ci::CiStatus,
    pub url: Option<String>,
    /// Merged pull requests, each in its own workspace, whose merge commit
    /// is an ancestor of `sha`: a deploy of a later commit shipped them too.
    pub contains: Vec<DeployedPull>,
}

/// A tracked pull request, named by the workspace that tracks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployedPull {
    pub workspace_id: crate::WorkspaceId,
    pub pull_number: i64,
}

/// A merged pull request a deploy of a later commit might have shipped:
/// tracked in `workspace_id`, merged as `merge_sha`, and not yet deployed to
/// the environment asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployCandidate {
    pub workspace_id: crate::WorkspaceId,
    pub pull_number: i64,
    pub merge_sha: String,
}

/// A stale-base notice for the thread a round was delivered on, said once per
/// delivered commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStaleBaseNotice {
    pub thread_id: ThreadId,
    pub repo: String,
    pub pull_number: i64,
    pub commit_sha: String,
    pub base_branch: Option<String>,
    pub base_behind: i64,
}

/// A recorded round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChangeRound {
    pub round: i64,
    pub commit_sha: String,
    pub base_behind: Option<i64>,
    pub diff_lines: i64,
    pub delivered_at: DateTime<Utc>,
    /// Whether this round rebuilt the branch on the base's head.
    pub rebase: bool,
    /// The head a squash-rebase round force-moved the branch from: with
    /// `commit_sha`, the range a reviewer diffs to see what the rebase
    /// changed. `None` for a normal round.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_head: Option<String>,
}

/// The label that marks a registered (session) pull request ready: the
/// change sweep stamps its time to ready when it first sees it.
pub const READY_LABEL: &str = "ready-to-merge";

/// A pull request the change flow did not open (a session's), registered
/// on its task's thread so the swarm metrics track it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RegisterPull {
    /// `owner/name`.
    pub repo: String,
    pub pull_number: i64,
    /// When the task was claimed, for a claim said outside Maidan; a
    /// `CLAIM:` comment Maidan posted takes precedence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<DateTime<Utc>>,
}

/// A registered pull request as tracked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RegisteredPull {
    /// Lowercase `owner/name`.
    pub repo: String,
    pub pull_number: i64,
    /// Where its time to ready starts: the task's `CLAIM:` comment, else the
    /// `claimed_at` given, else when it was registered.
    pub opened_at: DateTime<Utc>,
}

/// How long after a merge the change sweep keeps checking whether it held.
pub const OUTCOME_WINDOW_DAYS: i64 = 30;

/// What happened to a merged change afterwards, as the change sweep read it
/// from GitHub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergedOutcome {
    /// The commit the merge made on the base: what a revert names.
    pub merge_commit_sha: Option<String>,
    /// When a revert of it landed on the base.
    pub reverted_at: Option<DateTime<Utc>>,
    /// The reverting pull request (`#12`) or commit.
    pub revert_ref: Option<String>,
    /// Later merged pull requests into the same base that say they fix this
    /// one, or name the same Linear issue.
    pub followup_fixes: Vec<i64>,
    /// Commits on the pull request the change flow did not deliver; `None`
    /// until read, once, after the merge.
    pub human_commits: Option<i64>,
    /// Reviews and review comments by accounts that are not bots; `None`
    /// until read.
    pub human_reviews: Option<i64>,
    pub first_human_review_at: Option<DateTime<Utc>>,
}

/// A merged pull request due an outcome check: merged within
/// [`OUTCOME_WINDOW_DAYS`], not checked in the last day, in a workspace whose
/// last batch was over an hour ago.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeDue {
    /// Lowercase `owner/name`.
    pub repo: String,
    pub pull_number: i64,
    pub merged_at: DateTime<Utc>,
    /// The branch it merged into, from its newest round.
    pub base_branch: Option<String>,
    pub linear_issue: Option<String>,
    /// Every commit the change flow delivered to it.
    pub round_commits: Vec<String>,
    /// What earlier checks found.
    pub outcome: MergedOutcome,
}

/// An agent pull request the change flow opened, with its rounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePull {
    pub thread_id: ThreadId,
    pub repo: String,
    pub pull_number: i64,
    pub opened_at: DateTime<Utc>,
    pub ready_at: Option<DateTime<Utc>>,
    pub merged_at: Option<DateTime<Utc>>,
    pub merged_head_sha: Option<String>,
    pub closed_at: Option<DateTime<Utc>>,
    pub rounds: Vec<ChangeRound>,
    /// What the thread's model use cost, from the usage ledger.
    pub cost_usd_micros: i64,
    /// The provider and the harness that spent the most on the thread, from
    /// the usage its claimants reported; `None` when none was named.
    pub provider: Option<String>,
    pub harness: Option<String>,
    /// The task's attributes; the defaults when the thread was never
    /// described.
    pub attributes: crate::TaskAttributes,
    /// The Linear issue the change is for, when known.
    pub linear_issue: Option<String>,
    /// What happened after the merge.
    pub outcome: MergedOutcome,
    /// When it was handed to a session at `non_converging`, if it was.
    pub handed_off_at: Option<DateTime<Utc>>,
    /// The task's claim, review and delivery history, from the event log
    /// and its result deliveries.
    pub flow: ThreadFlow,
    /// How the checks stood on the head of the first round delivered: the
    /// first-try CI pass rate the scale test stops on. `None` while no CI
    /// notice names that head.
    pub first_delivery_ci: Option<crate::pull_ci::CiStatus>,
}

/// What happened to a task's thread around its pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadFlow {
    /// The first time a claimant was assigned the thread.
    pub claimed_at: Option<DateTime<Utc>>,
    /// The first move to `in_review`.
    pub review_requested_at: Option<DateTime<Utc>>,
    /// The first time a head's checks finished green (`checks_done`,
    /// success): when the work was really ready for a reviewer.
    pub ci_green_at: Option<DateTime<Utc>>,
    /// The first review submitted.
    pub first_review_at: Option<DateTime<Utc>>,
    /// Reviews that sent the work back (`request_changes`).
    pub sent_back: i64,
    /// Claims on the thread that lapsed (`ClaimExpired`).
    pub claim_lapses: i64,
    /// Result deliveries recorded as skipped or failed.
    pub deliveries_skipped: i64,
}

/// A task the change sweep handed to a session: the `ThreadHandoff` event
/// to publish, and who posts the message that says it (the thread's owner,
/// else its claim holder before the handoff, else whoever produced its
/// result).
#[derive(Debug, Clone)]
pub struct HandedOff {
    pub thread_id: ThreadId,
    pub workspace_id: WorkspaceId,
    pub event: StoredEvent,
    pub author_id: Option<MemberId>,
}

/// Why a task needs a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskFlag {
    /// Still open after the round limit, or its diff grew in each of the last
    /// three rounds: reroute it to a person or a session.
    NonConverging,
    /// A round was built on a base this many commits or more behind.
    StaleBase,
}

/// One task's numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskMetrics {
    pub thread_id: ThreadId,
    pub repo: String,
    pub pull_number: i64,
    pub opened_at: DateTime<Utc>,
    pub ready_at: Option<DateTime<Utc>>,
    pub merged_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub rounds: Vec<ChangeRound>,
    /// Opened to ready, in seconds.
    pub time_to_ready_secs: Option<i64>,
    /// Opened to merged, in seconds.
    pub time_to_merge_secs: Option<i64>,
    /// Whether the merged head was the last commit the change flow
    /// delivered, so nobody else pushed to it; `None` until merged.
    pub merged_without_edits: Option<bool>,
    pub cost_usd_micros: i64,
    /// The model provider that spent the most on the task (`anthropic`,
    /// `openai`, `xai`, ...), as its usage reports named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The harness that ran the work and spent the most (`claude-code`,
    /// `cursor`, `codex`, `pi`, ...), as its usage reports named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    pub flags: Vec<TaskFlag>,
    /// How many of the rounds were squash-rebases.
    pub rebases: i64,
    /// The task's attributes; the defaults when the thread was never
    /// described.
    pub attributes: crate::TaskAttributes,
    /// The Linear issue the change is for, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_issue: Option<String>,
    /// Whether a revert of it landed on the base; `None` until merged.
    pub reverted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_ref: Option<String>,
    /// Later merged pull requests that say they fix this one.
    pub followup_fixes: Vec<i64>,
    /// Commits on the pull request the change flow did not deliver: a proxy
    /// for a person's edits. `None` until read after the merge.
    pub human_commits: Option<i64>,
    /// Reviews and review comments by accounts that are not bots: a proxy
    /// for review effort, which GitHub cannot time.
    pub human_reviews: Option<i64>,
    /// From ready (else opened) to the first human review or review comment.
    pub time_to_first_human_review_secs: Option<i64>,
    /// Handed to a session at `non_converging`: counted as misrouted.
    pub handed_off: bool,
    /// When the thread was first assigned: a claim, or a person assigning
    /// it by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<DateTime<Utc>>,
    /// First claim to the first round delivered, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_to_first_delivery_secs: Option<i64>,
    /// How the checks stood on the first delivered head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_delivery_ci: Option<crate::pull_ci::CiStatus>,
    /// From the first green CI on a head (else the first move to review) to
    /// the first review submitted, in seconds: how long finished work waited
    /// for a reviewer, not for CI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_wait_secs: Option<i64>,
    /// Reviews that sent the work back.
    pub sent_back: i64,
    /// Claims that lapsed before the work was handed in.
    pub claim_lapses: i64,
    /// Result deliveries skipped or failed.
    pub deliveries_skipped: i64,
}

/// How a set of tasks did: a whole repository, or one slice of it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SliceNumbers {
    pub tasks: i64,
    pub merged: i64,
    pub merged_without_edits: i64,
    /// Tasks the engine they were routed to could not finish: flagged
    /// non-converging while open, or closed unmerged after reaching the
    /// round limit.
    pub misrouted: i64,
    /// `misrouted` over `tasks`; 0 with no tasks.
    pub misroute_rate: f64,
    pub mean_rounds: f64,
    /// Total cost of the tasks over the merged ones; `None` before the first
    /// merge.
    pub cost_usd_micros_per_merged: Option<i64>,
    /// Merged tasks a revert undid.
    pub reverted: i64,
    /// `reverted` over the merged tasks whose outcome is known: reverted,
    /// followed by a fix, or merged over [`OUTCOME_WINDOW_DAYS`] ago. `None`
    /// with none known.
    pub revert_rate: Option<f64>,
    /// Merged tasks a later pull request said it fixed.
    pub followup_fixed: i64,
    /// `followup_fixed` over the same known merged tasks.
    pub followup_fix_rate: Option<f64>,
    /// Mean commits a person added, over merged tasks read so far.
    pub mean_human_commits: Option<f64>,
    pub median_time_to_first_human_review_secs: Option<i64>,
    /// Tasks whose first delivered head passed CI, over those whose first
    /// head's checks finished; `None` with none finished.
    pub first_ci_pass_rate: Option<f64>,
    pub median_claim_to_first_delivery_secs: Option<i64>,
    pub median_review_wait_secs: Option<i64>,
    /// Reviews that sent work back, per task.
    pub mean_sent_back: f64,
    pub claim_lapses: i64,
    pub deliveries_skipped: i64,
}

/// The tasks routed to one engine.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EngineRollup {
    /// The engine the tasks were routed to; `None` for tasks never routed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<crate::TaskEngine>,
    #[serde(flatten)]
    pub numbers: SliceNumbers,
}

/// The tasks whose spend was mostly with one model provider.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProviderRollup {
    /// `None` for tasks whose usage named no provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(flatten)]
    pub numbers: SliceNumbers,
}

/// The tasks mostly run by one harness.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HarnessRollup {
    /// `None` for tasks whose usage named no harness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(flatten)]
    pub numbers: SliceNumbers,
}

/// The tasks of one size.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SizeRollup {
    /// The size the planner gave the tasks; `None` for tasks never sized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<crate::TaskSize>,
    #[serde(flatten)]
    pub numbers: SliceNumbers,
}

/// The tasks of one review tier.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TierRollup {
    /// The review tier; a task with no tier stored counts as `B`.
    pub tier: crate::TaskTier,
    #[serde(flatten)]
    pub numbers: SliceNumbers,
}

/// The tasks of one risk tier.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RiskRollup {
    /// Whether the tasks were marked as touching the threat model.
    pub threat_model: bool,
    #[serde(flatten)]
    pub numbers: SliceNumbers,
}

/// One repository's rollup. Pi is given small and medium work and sessions
/// the risky work, so an engine comparison is read within a size or a risk
/// tier, not across the whole repository.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RepoRollup {
    pub repo: String,
    #[serde(flatten)]
    pub totals: SliceNumbers,
    pub closed_unmerged: i64,
    pub flagged: i64,
    pub median_time_to_ready_secs: Option<i64>,
    /// By the engine each task was routed to, in engine order with unrouted
    /// tasks last.
    pub by_engine: Vec<EngineRollup>,
    /// By size, `S` to `L` with unsized tasks last.
    pub by_size: Vec<SizeRollup>,
    /// By risk tier, standard first. Slices with no tasks are omitted from
    /// every split.
    pub by_risk: Vec<RiskRollup>,
    /// By review tier, `A` then `B`, with untiered tasks counted as `B`.
    pub by_tier: Vec<TierRollup>,
    /// By the provider that spent the most on each task, most tasks first,
    /// unnamed last: so providers are compared as well as engines.
    pub by_provider: Vec<ProviderRollup>,
    /// By the harness that ran each task, most tasks first, unnamed last.
    pub by_harness: Vec<HarnessRollup>,
}

/// One usage-ledger group: what a thread's reports for one provider and
/// harness cost, and how many there were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendRow {
    pub thread_id: ThreadId,
    pub provider: Option<String>,
    pub harness: Option<String>,
    pub usd_micros: i64,
    pub reports: i64,
}

/// A thread's spend: its total, and who spent the most of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadSpend {
    pub cost_usd_micros: i64,
    pub provider: Option<String>,
    pub harness: Option<String>,
}

/// Each thread's [`ThreadSpend`]. The provider (and the harness) is the
/// named one with the most spend, then the most reports, then the first by
/// name, so the answer does not depend on row order; unpriced usage still
/// names one by its report count.
pub fn thread_spend(rows: impl IntoIterator<Item = SpendRow>) -> HashMap<ThreadId, ThreadSpend> {
    type Tally = HashMap<String, (i64, i64)>;
    let mut by_thread: HashMap<ThreadId, (i64, Tally, Tally)> = HashMap::new();
    for row in rows {
        let (total, providers, harnesses) = by_thread.entry(row.thread_id).or_default();
        *total = total.saturating_add(row.usd_micros);
        for (name, tally) in [
            (row.provider, &mut *providers),
            (row.harness, &mut *harnesses),
        ] {
            if let Some(name) = name {
                let entry = tally.entry(name).or_default();
                entry.0 = entry.0.saturating_add(row.usd_micros);
                entry.1 = entry.1.saturating_add(row.reports);
            }
        }
    }
    let top = |tally: Tally| {
        tally
            .into_iter()
            .max_by(|(a_name, a), (b_name, b)| a.cmp(b).then_with(|| b_name.cmp(a_name)))
            .map(|(name, _)| name)
    };
    by_thread
        .into_iter()
        .map(|(thread, (cost_usd_micros, providers, harnesses))| {
            (
                thread,
                ThreadSpend {
                    cost_usd_micros,
                    provider: top(providers),
                    harness: top(harnesses),
                },
            )
        })
        .collect()
}

/// The distinct values `key` takes over `tasks`, most tasks first, then by
/// name, with `None` last.
fn by_count(
    tasks: &[&TaskMetrics],
    key: fn(&TaskMetrics) -> Option<&String>,
) -> Vec<Option<String>> {
    let mut counts: HashMap<Option<String>, usize> = HashMap::new();
    for task in tasks {
        *counts.entry(key(task).cloned()).or_default() += 1;
    }
    let mut values: Vec<(Option<String>, usize)> = counts.into_iter().collect();
    values.sort_by(|(a, a_count), (b, b_count)| {
        a.is_none()
            .cmp(&b.is_none())
            .then(b_count.cmp(a_count))
            .then_with(|| a.cmp(b))
    });
    values.into_iter().map(|(value, _)| value).collect()
}

/// The ramp metrics for a workspace.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SwarmMetrics {
    pub tasks: Vec<TaskMetrics>,
    pub repos: Vec<RepoRollup>,
}

/// Thresholds for the flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagLimits {
    /// Rounds after which an unmerged task is non-converging.
    pub rounds: i64,
    /// Commits behind at which a round's base is stale.
    pub behind: i64,
}

impl Default for FlagLimits {
    fn default() -> Self {
        Self {
            rounds: 3,
            behind: 50,
        }
    }
}

/// The metrics for `pulls`, oldest first, with a rollup per repository, as
/// of `as_of` (which decides whose outcome window has run out).
pub fn swarm_metrics(
    pulls: Vec<ChangePull>,
    limits: FlagLimits,
    as_of: DateTime<Utc>,
) -> SwarmMetrics {
    let tasks: Vec<TaskMetrics> = pulls.into_iter().map(|p| task_metrics(p, limits)).collect();
    let mut repos: std::collections::BTreeMap<String, Vec<&TaskMetrics>> = Default::default();
    for task in &tasks {
        repos.entry(task.repo.clone()).or_default().push(task);
    }
    let repos = repos
        .into_iter()
        .map(|(repo, tasks)| rollup(repo, &tasks, limits.rounds, as_of))
        .collect();
    SwarmMetrics { tasks, repos }
}

fn task_metrics(pull: ChangePull, limits: FlagLimits) -> TaskMetrics {
    let secs = |at: Option<DateTime<Utc>>| at.map(|at| (at - pull.opened_at).num_seconds().max(0));
    let merged_without_edits =
        pull.merged_at.map(
            |_| match (pull.merged_head_sha.as_deref(), pull.rounds.last()) {
                (Some(head), Some(last)) => head.eq_ignore_ascii_case(&last.commit_sha),
                _ => false,
            },
        );
    let first_delivered = pull.rounds.first().map(|round| round.delivered_at);
    let mut flags = Vec::new();
    let open = pull.merged_at.is_none() && pull.closed_at.is_none();
    let growing = pull.rounds.len() >= 3
        && pull
            .rounds
            .windows(2)
            .rev()
            .take(2)
            // A squash-rebase round carries the whole branch's diff, so its
            // size says nothing about whether the rounds are converging.
            .all(|pair| !pair[1].rebase && pair[1].diff_lines > pair[0].diff_lines);
    if open && (pull.rounds.len() as i64 >= limits.rounds || growing) {
        flags.push(TaskFlag::NonConverging);
    }
    if pull
        .rounds
        .iter()
        .any(|r| r.base_behind.is_some_and(|b| b >= limits.behind))
    {
        flags.push(TaskFlag::StaleBase);
    }
    TaskMetrics {
        thread_id: pull.thread_id,
        repo: pull.repo,
        pull_number: pull.pull_number,
        opened_at: pull.opened_at,
        ready_at: pull.ready_at,
        merged_at: pull.merged_at,
        closed_at: pull.closed_at,
        time_to_ready_secs: secs(pull.ready_at),
        time_to_merge_secs: secs(pull.merged_at),
        merged_without_edits,
        cost_usd_micros: pull.cost_usd_micros,
        provider: pull.provider,
        harness: pull.harness,
        flags,
        rebases: pull.rounds.iter().filter(|r| r.rebase).count() as i64,
        rounds: pull.rounds,
        attributes: pull.attributes,
        linear_issue: pull.linear_issue,
        reverted: pull.merged_at.map(|_| pull.outcome.reverted_at.is_some()),
        revert_ref: pull.outcome.revert_ref,
        followup_fixes: pull.outcome.followup_fixes,
        human_commits: pull.outcome.human_commits,
        human_reviews: pull.outcome.human_reviews,
        time_to_first_human_review_secs: pull.outcome.first_human_review_at.map(|at| {
            (at - pull.ready_at.unwrap_or(pull.opened_at))
                .num_seconds()
                .max(0)
        }),
        handed_off: pull.handed_off_at.is_some(),
        claimed_at: pull.flow.claimed_at,
        claim_to_first_delivery_secs: pull
            .flow
            .claimed_at
            .zip(first_delivered)
            .map(|(claimed, delivered)| (delivered - claimed).num_seconds().max(0)),
        first_delivery_ci: pull.first_delivery_ci,
        review_wait_secs: pull
            .flow
            .ci_green_at
            .or(pull.flow.review_requested_at)
            .zip(pull.flow.first_review_at)
            .filter(|(asked, reviewed)| reviewed >= asked)
            .map(|(asked, reviewed)| (reviewed - asked).num_seconds()),
        sent_back: pull.flow.sent_back,
        claim_lapses: pull.flow.claim_lapses,
        deliveries_skipped: pull.flow.deliveries_skipped,
    }
}

/// One task per thread. The flow counts (sends back, lapses, skipped
/// deliveries) belong to the thread, and a thread that tracks two pull
/// requests carries them on both, so summing over tasks counts them twice.
fn per_thread<'a>(tasks: &[&'a TaskMetrics]) -> Vec<&'a TaskMetrics> {
    let mut seen = std::collections::HashSet::new();
    tasks
        .iter()
        .copied()
        .filter(|t| seen.insert(t.thread_id))
        .collect()
}

/// The middle value of `values`, which it sorts.
fn median(mut values: Vec<i64>) -> Option<i64> {
    values.sort_unstable();
    values.get(values.len() / 2).copied()
}

fn rollup(
    repo: String,
    tasks: &[&TaskMetrics],
    round_limit: i64,
    as_of: DateTime<Utc>,
) -> RepoRollup {
    let mut ready: Vec<i64> = tasks.iter().filter_map(|t| t.time_to_ready_secs).collect();
    ready.sort_unstable();
    let slice = |keep: &dyn Fn(&TaskMetrics) -> bool| -> Option<SliceNumbers> {
        let mine: Vec<&TaskMetrics> = tasks.iter().copied().filter(|t| keep(t)).collect();
        (!mine.is_empty()).then(|| numbers(&mine, round_limit, as_of))
    };
    let by_engine = [
        Some(crate::TaskEngine::Pi),
        Some(crate::TaskEngine::Session),
        Some(crate::TaskEngine::Human),
        None,
    ]
    .into_iter()
    .filter_map(|engine| {
        slice(&|t| t.attributes.engine == engine).map(|numbers| EngineRollup { engine, numbers })
    })
    .collect();
    let by_size = [
        Some(crate::TaskSize::S),
        Some(crate::TaskSize::M),
        Some(crate::TaskSize::L),
        None,
    ]
    .into_iter()
    .filter_map(|size| {
        slice(&|t| t.attributes.size == size).map(|numbers| SizeRollup { size, numbers })
    })
    .collect();
    let by_risk = [false, true]
        .into_iter()
        .filter_map(|threat_model| {
            slice(&|t| t.attributes.threat_model == threat_model).map(|numbers| RiskRollup {
                threat_model,
                numbers,
            })
        })
        .collect();
    let by_tier = [crate::TaskTier::A, crate::TaskTier::B]
        .into_iter()
        .filter_map(|tier| {
            slice(&|t| t.attributes.tier.unwrap_or(crate::TaskTier::B) == tier)
                .map(|numbers| TierRollup { tier, numbers })
        })
        .collect();
    let by_provider = by_count(tasks, |t| t.provider.as_ref())
        .into_iter()
        .filter_map(|provider| {
            slice(&|t| t.provider == provider).map(|numbers| ProviderRollup { provider, numbers })
        })
        .collect();
    let by_harness = by_count(tasks, |t| t.harness.as_ref())
        .into_iter()
        .filter_map(|harness| {
            slice(&|t| t.harness == harness).map(|numbers| HarnessRollup { harness, numbers })
        })
        .collect();
    RepoRollup {
        repo,
        totals: numbers(tasks, round_limit, as_of),
        closed_unmerged: tasks
            .iter()
            .filter(|t| t.merged_at.is_none() && t.closed_at.is_some())
            .count() as i64,
        flagged: tasks.iter().filter(|t| !t.flags.is_empty()).count() as i64,
        median_time_to_ready_secs: ready
            .get(ready.len() / 2)
            .copied()
            .filter(|_| !ready.is_empty()),
        by_engine,
        by_size,
        by_risk,
        by_tier,
        by_provider,
        by_harness,
    }
}

/// Whether the engine a task was routed to could not finish it: still open
/// and non-converging, or closed unmerged after reaching the round limit.
fn misrouted(task: &TaskMetrics, round_limit: i64) -> bool {
    task.handed_off
        || task.flags.contains(&TaskFlag::NonConverging)
        || (task.merged_at.is_none()
            && task.closed_at.is_some()
            && task.rounds.len() as i64 >= round_limit)
}

/// Whether a merged task's outcome is settled: something was found, or the
/// window to find it in has run out.
fn outcome_known(task: &TaskMetrics, as_of: DateTime<Utc>) -> bool {
    task.merged_at.is_some_and(|merged| {
        task.reverted == Some(true)
            || !task.followup_fixes.is_empty()
            || as_of - merged >= chrono::Duration::days(OUTCOME_WINDOW_DAYS)
    })
}

fn rate(hits: i64, of: i64) -> Option<f64> {
    (of > 0).then(|| hits as f64 / of as f64)
}

fn numbers(tasks: &[&TaskMetrics], round_limit: i64, as_of: DateTime<Utc>) -> SliceNumbers {
    let count = tasks.len() as i64;
    let known = tasks.iter().filter(|t| outcome_known(t, as_of)).count() as i64;
    let reverted = tasks.iter().filter(|t| t.reverted == Some(true)).count() as i64;
    let followup_fixed = tasks
        .iter()
        .filter(|t| t.merged_at.is_some() && !t.followup_fixes.is_empty())
        .count() as i64;
    let human: Vec<i64> = tasks
        .iter()
        .filter(|t| t.merged_at.is_some())
        .filter_map(|t| t.human_commits)
        .collect();
    let mut first_review: Vec<i64> = tasks
        .iter()
        .filter_map(|t| t.time_to_first_human_review_secs)
        .collect();
    first_review.sort_unstable();
    let merged = tasks.iter().filter(|t| t.merged_at.is_some()).count() as i64;
    let misrouted = tasks.iter().filter(|t| misrouted(t, round_limit)).count() as i64;
    let cost: i64 = tasks.iter().map(|t| t.cost_usd_micros).sum();
    let per_task = |total: f64| {
        if count == 0 {
            0.0
        } else {
            total / count as f64
        }
    };
    SliceNumbers {
        tasks: count,
        merged,
        merged_without_edits: tasks
            .iter()
            .filter(|t| t.merged_without_edits == Some(true))
            .count() as i64,
        misrouted,
        misroute_rate: per_task(misrouted as f64),
        mean_rounds: per_task(tasks.iter().map(|t| t.rounds.len() as f64).sum()),
        cost_usd_micros_per_merged: (merged > 0).then(|| cost / merged),
        reverted,
        revert_rate: rate(reverted, known),
        followup_fixed,
        followup_fix_rate: rate(followup_fixed, known),
        mean_human_commits: (!human.is_empty())
            .then(|| human.iter().sum::<i64>() as f64 / human.len() as f64),
        median_time_to_first_human_review_secs: first_review.get(first_review.len() / 2).copied(),
        first_ci_pass_rate: {
            let finished: Vec<crate::pull_ci::CiStatus> = tasks
                .iter()
                .filter_map(|t| t.first_delivery_ci)
                .filter(|status| *status != crate::pull_ci::CiStatus::Pending)
                .collect();
            let passed = finished
                .iter()
                .filter(|status| **status == crate::pull_ci::CiStatus::Success)
                .count() as i64;
            rate(passed, finished.len() as i64)
        },
        median_claim_to_first_delivery_secs: median(
            tasks
                .iter()
                .filter_map(|t| t.claim_to_first_delivery_secs)
                .collect(),
        ),
        median_review_wait_secs: median(tasks.iter().filter_map(|t| t.review_wait_secs).collect()),
        mean_sent_back: {
            let threads = per_thread(tasks);
            if threads.is_empty() {
                0.0
            } else {
                threads.iter().map(|t| t.sent_back as f64).sum::<f64>() / threads.len() as f64
            }
        },
        claim_lapses: per_thread(tasks).iter().map(|t| t.claim_lapses).sum(),
        deliveries_skipped: per_thread(tasks).iter().map(|t| t.deliveries_skipped).sum(),
    }
}

/// Added plus removed lines in a git diff, headers excluded.
pub fn diff_lines(diff: &str) -> i64 {
    diff.lines()
        .filter(|l| {
            (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"))
        })
        .count() as i64
}

/// The files a git diff touches: each `--- a/` and `+++ b/` path except
/// `/dev/null`, first seen first, deduplicated, at most
/// [`ROUND_PATH_LIMIT`].
pub fn diff_paths(diff: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();
    for line in diff.lines() {
        let Some(path) = line
            .strip_prefix("--- a/")
            .or_else(|| line.strip_prefix("+++ b/"))
        else {
            continue;
        };
        // git appends a tab before a timestamp on some diffs.
        let path = path.split('\t').next().unwrap_or(path).trim_end();
        if path.is_empty() || !seen.insert(path.to_string()) {
            continue;
        }
        paths.push(path.to_string());
        if paths.len() == ROUND_PATH_LIMIT {
            break;
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + secs, 0).unwrap()
    }

    fn round(n: i64, sha: &str, diff: i64, behind: Option<i64>) -> ChangeRound {
        ChangeRound {
            round: n,
            commit_sha: sha.into(),
            base_behind: behind,
            diff_lines: diff,
            delivered_at: at(n * 60),
            rebase: false,
            previous_head: None,
        }
    }

    fn pull(rounds: Vec<ChangeRound>) -> ChangePull {
        ChangePull {
            thread_id: ThreadId(uuid::Uuid::nil()),
            repo: "example/app".into(),
            pull_number: 7,
            opened_at: at(0),
            ready_at: None,
            merged_at: None,
            merged_head_sha: None,
            closed_at: None,
            rounds,
            cost_usd_micros: 300,
            provider: None,
            harness: None,
            attributes: crate::TaskAttributes::default(),
            linear_issue: None,
            outcome: MergedOutcome::default(),
            handed_off_at: None,
            flow: ThreadFlow::default(),
            first_delivery_ci: None,
        }
    }

    #[test]
    fn a_merge_at_the_delivered_head_counts_as_merged_without_edits() {
        let mut clean = pull(vec![
            round(1, "aaa", 10, Some(0)),
            round(2, "bbb", 4, Some(1)),
        ]);
        clean.ready_at = Some(at(600));
        clean.merged_at = Some(at(900));
        clean.merged_head_sha = Some("BBB".into());
        let mut edited = pull(vec![round(1, "ccc", 10, None)]);
        edited.merged_at = Some(at(100));
        edited.merged_head_sha = Some("ddd".into());
        let metrics = swarm_metrics(vec![clean, edited], FlagLimits::default(), at(0));
        assert_eq!(metrics.tasks[0].merged_without_edits, Some(true));
        assert_eq!(metrics.tasks[0].time_to_ready_secs, Some(600));
        assert_eq!(metrics.tasks[0].time_to_merge_secs, Some(900));
        assert_eq!(metrics.tasks[1].merged_without_edits, Some(false));
        let repo = &metrics.repos[0];
        let totals = &repo.totals;
        assert_eq!(
            (totals.tasks, totals.merged, totals.merged_without_edits),
            (2, 2, 1)
        );
        assert_eq!(totals.mean_rounds, 1.5);
        assert_eq!(totals.cost_usd_micros_per_merged, Some(300));
        assert_eq!(repo.median_time_to_ready_secs, Some(600));
    }

    #[test]
    fn rounds_past_the_limit_or_a_growing_diff_flag_an_open_task() {
        let limits = FlagLimits::default();
        let many = pull((1..=3).map(|n| round(n, "x", 5, Some(0))).collect());
        let growing = pull(vec![
            round(1, "a", 10, None),
            round(2, "b", 20, None),
            round(3, "c", 40, None),
        ]);
        let shrinking = pull(vec![round(1, "a", 40, None), round(2, "b", 20, None)]);
        let mut merged = pull((1..=5).map(|n| round(n, "x", 5, None)).collect());
        merged.merged_at = Some(at(10));
        let stale = pull(vec![round(1, "a", 3, Some(331))]);
        let m = swarm_metrics(vec![many, growing, shrinking, merged, stale], limits, at(0));
        assert_eq!(m.tasks[0].flags, vec![TaskFlag::NonConverging]);
        assert_eq!(m.tasks[1].flags, vec![TaskFlag::NonConverging]);
        assert!(m.tasks[2].flags.is_empty());
        assert!(m.tasks[3].flags.is_empty(), "a merged task is done");
        assert_eq!(m.tasks[4].flags, vec![TaskFlag::StaleBase]);
        assert_eq!(m.repos[0].flagged, 3);
    }

    #[test]
    fn rebase_rounds_are_counted_per_task_and_never_read_as_a_growing_diff() {
        let mut rebased = round(3, "c", 40, Some(0));
        rebased.rebase = true;
        let task = pull(vec![
            round(1, "a", 10, None),
            round(2, "b", 20, None),
            rebased,
        ]);
        let m = swarm_metrics(
            vec![task],
            FlagLimits {
                rounds: 10,
                ..FlagLimits::default()
            },
            at(0),
        );
        assert_eq!(m.tasks[0].rebases, 1);
        assert!(m.tasks[0].flags.is_empty(), "{:?}", m.tasks[0].flags);
    }

    fn routed(
        engine: Option<crate::TaskEngine>,
        size: Option<crate::TaskSize>,
        threat_model: bool,
        rounds: i64,
    ) -> ChangePull {
        let mut p = pull((1..=rounds).map(|n| round(n, "x", 5, None)).collect());
        p.attributes = crate::TaskAttributes {
            engine,
            size,
            threat_model,
            ..crate::TaskAttributes::default()
        };
        p
    }

    /// The scale test's numbers: claim to first delivery, the first head's
    /// CI, review wait from the first green CI (else the move to review),
    /// work sent back, lapses and skipped deliveries, per task and per slice.
    /// A thread that tracks two pull requests counts its sends back, lapses
    /// and skipped deliveries once in a slice, not once per pull request.
    #[test]
    fn a_thread_with_two_pulls_counts_its_flow_once() {
        let mut first = pull(vec![round(1, "aaa", 10, Some(0))]);
        first.flow = ThreadFlow {
            sent_back: 2,
            claim_lapses: 1,
            deliveries_skipped: 3,
            ..ThreadFlow::default()
        };
        let mut second = first.clone();
        second.repo = "example/other".into();
        second.pull_number = 9;
        let m = swarm_metrics(
            vec![first.clone(), second],
            FlagLimits::default(),
            at(2_000),
        );
        let all: Vec<&TaskMetrics> = m.tasks.iter().collect();
        let n = numbers(&all, FlagLimits::default().rounds, at(2_000));
        assert_eq!(n.claim_lapses, 1);
        assert_eq!(n.deliveries_skipped, 3);
        assert_eq!(n.mean_sent_back, 2.0);
    }

    #[test]
    fn each_task_and_slice_carries_the_scale_test_numbers() {
        let mut quick = pull(vec![round(1, "aaa", 10, Some(0))]);
        quick.flow = ThreadFlow {
            claimed_at: Some(at(-600)),
            review_requested_at: Some(at(100)),
            ci_green_at: Some(at(400)),
            first_review_at: Some(at(1_000)),
            sent_back: 1,
            claim_lapses: 1,
            deliveries_skipped: 2,
        };
        quick.first_delivery_ci = Some(crate::pull_ci::CiStatus::Success);
        let mut red = pull(vec![round(1, "bbb", 10, Some(0))]);
        red.pull_number = 8;
        red.thread_id = ThreadId(uuid::Uuid::from_u128(2));
        red.flow = ThreadFlow {
            review_requested_at: Some(at(0)),
            first_review_at: Some(at(300)),
            ..ThreadFlow::default()
        };
        red.first_delivery_ci = Some(crate::pull_ci::CiStatus::Failure);
        let mut pending = pull(vec![round(1, "ccc", 10, Some(0))]);
        pending.pull_number = 9;
        pending.thread_id = ThreadId(uuid::Uuid::from_u128(3));
        pending.first_delivery_ci = Some(crate::pull_ci::CiStatus::Pending);
        let delivered = quick.rounds[0].delivered_at;

        let m = swarm_metrics(vec![quick, red, pending], FlagLimits::default(), at(2_000));
        let first = &m.tasks[0];
        assert_eq!(
            first.claim_to_first_delivery_secs,
            Some((delivered - at(-600)).num_seconds())
        );
        assert_eq!(
            first.review_wait_secs,
            Some(600),
            "from the green CI, not the move to review"
        );
        assert_eq!(
            m.tasks[1].review_wait_secs,
            Some(300),
            "no CI record: from the move to review"
        );
        assert_eq!(m.tasks[2].review_wait_secs, None);
        let totals = &m.repos[0].totals;
        assert_eq!(
            totals.first_ci_pass_rate,
            Some(0.5),
            "pending heads are left out"
        );
        assert_eq!(totals.mean_sent_back, 1.0 / 3.0);
        assert_eq!(totals.claim_lapses, 1);
        assert_eq!(totals.deliveries_skipped, 2);
        assert_eq!(totals.median_review_wait_secs, Some(600));
    }

    /// A thread's provider and harness are the ones that spent the most;
    /// unpriced reports break the tie by count, then by name, and unnamed
    /// usage still counts toward the total.
    #[test]
    fn a_threads_provider_is_the_one_that_spent_the_most() {
        let t = ThreadId(uuid::Uuid::from_u128(1));
        let u = ThreadId(uuid::Uuid::from_u128(2));
        let row = |thread, provider: Option<&str>, harness: Option<&str>, usd, reports| SpendRow {
            thread_id: thread,
            provider: provider.map(str::to_string),
            harness: harness.map(str::to_string),
            usd_micros: usd,
            reports,
        };
        let spend = thread_spend([
            row(t, Some("anthropic"), Some("claude-code"), 500, 1),
            row(t, Some("openai"), Some("codex"), 900, 1),
            row(t, None, None, 50, 3),
            row(u, Some("xai"), Some("openrouter"), 0, 2),
            row(u, Some("anthropic"), Some("cursor"), 0, 2),
        ]);
        assert_eq!(
            spend[&t],
            ThreadSpend {
                cost_usd_micros: 1_450,
                provider: Some("openai".into()),
                harness: Some("codex".into()),
            }
        );
        assert_eq!(spend[&u].provider.as_deref(), Some("anthropic"));
        assert_eq!(spend[&u].harness.as_deref(), Some("cursor"));
    }

    /// Each repository's rollup is split by provider and by harness, most
    /// tasks first and unnamed last, so a provider comparison reads beside
    /// the engine one.
    #[test]
    fn each_rollup_splits_by_provider_and_harness() {
        let named = |provider: Option<&str>, harness: Option<&str>| {
            let mut p = pull(vec![round(1, "x", 5, None)]);
            p.provider = provider.map(str::to_string);
            p.harness = harness.map(str::to_string);
            p
        };
        let m = swarm_metrics(
            vec![
                named(Some("xai"), Some("openrouter")),
                named(Some("anthropic"), Some("claude-code")),
                named(Some("anthropic"), Some("cursor")),
                named(None, None),
            ],
            FlagLimits::default(),
            at(0),
        );
        let repo = &m.repos[0];
        let providers: Vec<_> = repo
            .by_provider
            .iter()
            .map(|r| (r.provider.as_deref(), r.numbers.tasks))
            .collect();
        assert_eq!(
            providers,
            vec![(Some("anthropic"), 2), (Some("xai"), 1), (None, 1)]
        );
        let harnesses: Vec<_> = repo
            .by_harness
            .iter()
            .map(|r| r.harness.as_deref())
            .collect();
        assert_eq!(
            harnesses,
            vec![
                Some("claude-code"),
                Some("cursor"),
                Some("openrouter"),
                None
            ]
        );
        assert_eq!(m.tasks[0].provider.as_deref(), Some("xai"));
    }

    #[test]
    fn by_tier_counts_an_untiered_task_as_b() {
        use crate::TaskTier;
        let tiered = |tier: Option<TaskTier>, merged: bool| {
            let mut p = routed(None, None, false, 1);
            p.attributes.tier = tier;
            if merged {
                p.merged_at = Some(at(100));
            }
            p
        };
        let m = swarm_metrics(
            vec![
                tiered(Some(TaskTier::A), true),
                tiered(Some(TaskTier::B), false),
                tiered(None, true),
            ],
            FlagLimits::default(),
            at(0),
        );
        let tiers: Vec<_> = m.repos[0]
            .by_tier
            .iter()
            .map(|r| (r.tier, r.numbers.tasks, r.numbers.merged))
            .collect();
        assert_eq!(tiers, vec![(TaskTier::A, 1, 1), (TaskTier::B, 2, 1)]);

        let only_b = swarm_metrics(vec![tiered(None, false)], FlagLimits::default(), at(0));
        let tiers: Vec<_> = only_b.repos[0].by_tier.iter().map(|r| r.tier).collect();
        assert_eq!(tiers, vec![TaskTier::B], "a tier with no tasks is left out");
        let json = serde_json::to_value(&only_b.repos[0]).unwrap();
        assert_eq!(json["by_tier"][0]["tier"], "B");
    }

    #[test]
    fn each_rollup_splits_by_engine_size_and_risk_with_cost_and_misroutes() {
        use crate::{TaskEngine, TaskSize};
        // Pi, small, merged after one round at 300.
        let mut small = routed(Some(TaskEngine::Pi), Some(TaskSize::S), false, 1);
        small.merged_at = Some(at(100));
        // Pi, medium, still open at the round limit: misrouted.
        let stuck = routed(Some(TaskEngine::Pi), Some(TaskSize::M), false, 3);
        // A session, threat-model work, closed unmerged at the round limit:
        // misrouted too.
        let mut abandoned = routed(Some(TaskEngine::Session), Some(TaskSize::L), true, 3);
        abandoned.closed_at = Some(at(500));
        // A session, threat-model work, merged; never sized.
        let mut risky = routed(Some(TaskEngine::Session), None, true, 2);
        risky.merged_at = Some(at(900));
        risky.cost_usd_micros = 900;
        // Closed unmerged early: given up on, not misrouted.
        let mut dropped = routed(None, Some(TaskSize::S), false, 1);
        dropped.closed_at = Some(at(50));

        let m = swarm_metrics(
            vec![small, stuck, abandoned, risky, dropped],
            FlagLimits::default(),
            at(0),
        );
        let repo = &m.repos[0];
        assert_eq!(repo.totals.tasks, 5);
        assert_eq!(repo.totals.merged, 2);
        assert_eq!(repo.totals.misrouted, 2);
        assert_eq!(repo.totals.misroute_rate, 0.4);
        assert_eq!(repo.totals.mean_rounds, 2.0);
        assert_eq!(
            repo.totals.cost_usd_micros_per_merged,
            Some((300 * 4 + 900) / 2)
        );

        let engines: Vec<_> = repo
            .by_engine
            .iter()
            .map(|e| {
                (
                    e.engine,
                    e.numbers.tasks,
                    e.numbers.merged,
                    e.numbers.misrouted,
                )
            })
            .collect();
        assert_eq!(
            engines,
            vec![
                (Some(TaskEngine::Pi), 2, 1, 1),
                (Some(TaskEngine::Session), 2, 1, 1),
                (None, 1, 0, 0),
            ],
            "an engine with no tasks is left out"
        );
        assert_eq!(
            repo.by_engine[0].numbers.cost_usd_micros_per_merged,
            Some(600)
        );
        assert_eq!(
            repo.by_engine[1].numbers.cost_usd_micros_per_merged,
            Some(1200)
        );
        assert_eq!(repo.by_engine[2].numbers.cost_usd_micros_per_merged, None);

        let sizes: Vec<_> = repo
            .by_size
            .iter()
            .map(|s| (s.size, s.numbers.tasks))
            .collect();
        assert_eq!(
            sizes,
            vec![
                (Some(TaskSize::S), 2),
                (Some(TaskSize::M), 1),
                (Some(TaskSize::L), 1),
                (None, 1),
            ]
        );
        let risk: Vec<_> = repo
            .by_risk
            .iter()
            .map(|r| (r.threat_model, r.numbers.tasks, r.numbers.misroute_rate))
            .collect();
        assert_eq!(risk, vec![(false, 3, 1.0 / 3.0), (true, 2, 0.5)]);
    }

    #[test]
    fn outcome_rates_count_only_merges_whose_outcome_is_known() {
        let day = 86_400;
        let merged = |at_secs: i64, outcome: MergedOutcome| {
            let mut p = pull(vec![round(1, "a", 5, None)]);
            p.ready_at = Some(at(at_secs - 1000));
            p.merged_at = Some(at(at_secs));
            p.outcome = outcome;
            p
        };
        let reverted = merged(
            0,
            MergedOutcome {
                reverted_at: Some(at(day)),
                revert_ref: Some("#12".into()),
                human_commits: Some(2),
                human_reviews: Some(3),
                first_human_review_at: Some(at(-900)),
                ..MergedOutcome::default()
            },
        );
        let fixed = merged(
            0,
            MergedOutcome {
                followup_fixes: vec![9],
                human_commits: Some(0),
                ..MergedOutcome::default()
            },
        );
        // Nothing found, and the window has run out: held.
        let held = merged(0, MergedOutcome::default());
        // Nothing found yet, two days in: not known, not in the rates.
        let recent = merged(
            29 * day,
            MergedOutcome {
                human_commits: Some(4),
                ..MergedOutcome::default()
            },
        );
        let open = pull(vec![round(1, "a", 5, None)]);
        let m = swarm_metrics(
            vec![reverted, fixed, held, recent, open],
            FlagLimits::default(),
            at(31 * day),
        );
        assert_eq!(m.tasks[0].reverted, Some(true));
        assert_eq!(m.tasks[0].revert_ref.as_deref(), Some("#12"));
        assert_eq!(m.tasks[0].time_to_first_human_review_secs, Some(100));
        assert_eq!(m.tasks[1].followup_fixes, vec![9]);
        assert_eq!(m.tasks[2].reverted, Some(false));
        assert_eq!(m.tasks[4].reverted, None, "not merged");
        let totals = &m.repos[0].totals;
        assert_eq!((totals.reverted, totals.followup_fixed), (1, 1));
        assert_eq!(totals.revert_rate, Some(1.0 / 3.0));
        assert_eq!(totals.followup_fix_rate, Some(1.0 / 3.0));
        assert_eq!(totals.mean_human_commits, Some(2.0));
        assert_eq!(totals.median_time_to_first_human_review_secs, Some(100));

        let early = swarm_metrics(
            vec![merged(0, MergedOutcome::default())],
            FlagLimits::default(),
            at(day),
        );
        assert_eq!(early.repos[0].totals.revert_rate, None, "nothing known yet");
    }

    #[test]
    fn a_handed_off_task_counts_as_misrouted() {
        let mut handed = pull(vec![round(1, "a", 5, None)]);
        handed.handed_off_at = Some(at(100));
        let m = swarm_metrics(
            vec![handed, pull(vec![round(1, "b", 5, None)])],
            FlagLimits::default(),
            at(0),
        );
        assert!(m.tasks[0].handed_off);
        assert_eq!(m.repos[0].totals.misrouted, 1);
    }

    #[test]
    fn a_slice_serializes_flat_beside_its_key() {
        let mut task = routed(
            Some(crate::TaskEngine::Pi),
            Some(crate::TaskSize::M),
            false,
            1,
        );
        task.merged_at = Some(at(10));
        let m = swarm_metrics(vec![task], FlagLimits::default(), at(0));
        let json = serde_json::to_value(&m.repos[0]).unwrap();
        assert_eq!(json["tasks"], 1);
        assert_eq!(json["misroute_rate"], 0.0);
        assert_eq!(json["by_engine"][0]["engine"], "pi");
        assert_eq!(json["by_engine"][0]["merged"], 1);
        assert_eq!(json["by_size"][0]["size"], "M");
        assert_eq!(json["by_risk"][0]["threat_model"], false);
        assert!(json.get("totals").is_none(), "{json}");
    }

    #[test]
    fn diff_paths_name_each_touched_file_once_and_skip_dev_null() {
        let diff = "--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1 +1 @@\n-a\n+b\n\
                    --- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1 @@\n+c\n\
                    --- a/src/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-d\n";
        assert_eq!(
            diff_paths(diff),
            vec!["src/x.rs", "src/new.rs", "src/gone.rs"]
        );
        let many: String = (0..600)
            .map(|n| format!("--- a/f{n}\n+++ b/f{n}\n"))
            .collect();
        assert_eq!(diff_paths(&many).len(), ROUND_PATH_LIMIT);
    }

    #[test]
    fn diff_lines_count_changes_not_headers() {
        let diff = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-old\n+new\n same\n+added\n";
        assert_eq!(diff_lines(diff), 3);
    }
}
