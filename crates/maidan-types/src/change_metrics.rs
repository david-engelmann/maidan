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
            ThreadNoticeKind::GateClear => format!(
                "{}#{} is no longer held back.",
                self.repo, self.pull_number
            ),
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
    /// The task's attributes; the defaults when the thread was never
    /// described.
    pub attributes: crate::TaskAttributes,
    /// The Linear issue the change is for, when known.
    pub linear_issue: Option<String>,
    /// What happened after the merge.
    pub outcome: MergedOutcome,
    /// When it was handed to a session at `non_converging`, if it was.
    pub handed_off_at: Option<DateTime<Utc>>,
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
    /// all three splits.
    pub by_risk: Vec<RiskRollup>,
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
    }
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
            attributes: crate::TaskAttributes::default(),
            linear_issue: None,
            outcome: MergedOutcome::default(),
            handed_off_at: None,
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
