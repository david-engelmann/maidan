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
}

impl ThreadNoticeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RebaseNeeded => "rebase_needed",
            Self::StaleBase => "stale_base",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rebase_needed" => Some(Self::RebaseNeeded),
            "stale_base" => Some(Self::StaleBase),
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
    /// The merged pull request (`rebase_needed`) or the one the stale round
    /// went to (`stale_base`).
    pub pull_number: i64,
    /// The merged paths the claim covers, at most 20 ([`NOTICE_PATH_LIMIT`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_behind: Option<i64>,
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
        }
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
    /// The task's attributes; the defaults when the thread was never
    /// described.
    pub attributes: crate::TaskAttributes,
}

/// One engine's slice of a repository rollup: how much of the repo's work
/// was routed to it, and how much of that merged.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EngineRollup {
    /// The engine the tasks were routed to; `None` for tasks never routed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<crate::TaskEngine>,
    pub tasks: i64,
    pub merged: i64,
}

/// One repository's rollup.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RepoRollup {
    pub repo: String,
    pub tasks: i64,
    pub merged: i64,
    pub merged_without_edits: i64,
    pub closed_unmerged: i64,
    pub flagged: i64,
    pub mean_rounds: f64,
    pub median_time_to_ready_secs: Option<i64>,
    /// Total cost of the repository's tasks over its merged ones; `None`
    /// before the first merge.
    pub cost_usd_micros_per_merged: Option<i64>,
    /// The rollup split by the engine each task was routed to, in engine
    /// order with unrouted tasks last; engines with no tasks are omitted.
    pub by_engine: Vec<EngineRollup>,
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

/// The metrics for `pulls`, oldest first, with a rollup per repository.
pub fn swarm_metrics(pulls: Vec<ChangePull>, limits: FlagLimits) -> SwarmMetrics {
    let tasks: Vec<TaskMetrics> = pulls.into_iter().map(|p| task_metrics(p, limits)).collect();
    let mut repos: std::collections::BTreeMap<String, Vec<&TaskMetrics>> = Default::default();
    for task in &tasks {
        repos.entry(task.repo.clone()).or_default().push(task);
    }
    let repos = repos
        .into_iter()
        .map(|(repo, tasks)| rollup(repo, &tasks))
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
            .all(|pair| pair[1].diff_lines > pair[0].diff_lines);
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
        rounds: pull.rounds,
        attributes: pull.attributes,
    }
}

fn rollup(repo: String, tasks: &[&TaskMetrics]) -> RepoRollup {
    let merged = tasks.iter().filter(|t| t.merged_at.is_some()).count() as i64;
    let mut ready: Vec<i64> = tasks.iter().filter_map(|t| t.time_to_ready_secs).collect();
    ready.sort_unstable();
    let cost: i64 = tasks.iter().map(|t| t.cost_usd_micros).sum();
    RepoRollup {
        repo,
        tasks: tasks.len() as i64,
        merged,
        merged_without_edits: tasks
            .iter()
            .filter(|t| t.merged_without_edits == Some(true))
            .count() as i64,
        closed_unmerged: tasks
            .iter()
            .filter(|t| t.merged_at.is_none() && t.closed_at.is_some())
            .count() as i64,
        flagged: tasks.iter().filter(|t| !t.flags.is_empty()).count() as i64,
        mean_rounds: if tasks.is_empty() {
            0.0
        } else {
            tasks.iter().map(|t| t.rounds.len() as f64).sum::<f64>() / tasks.len() as f64
        },
        median_time_to_ready_secs: ready
            .get(ready.len() / 2)
            .copied()
            .filter(|_| !ready.is_empty()),
        cost_usd_micros_per_merged: (merged > 0).then(|| cost / merged),
        by_engine: engine_split(tasks),
    }
}

/// The per-engine split of one repository's tasks, in engine order with
/// unrouted tasks last; engines with no tasks are omitted.
fn engine_split(tasks: &[&TaskMetrics]) -> Vec<EngineRollup> {
    const ORDER: [Option<crate::TaskEngine>; 4] = [
        Some(crate::TaskEngine::Pi),
        Some(crate::TaskEngine::Session),
        Some(crate::TaskEngine::Human),
        None,
    ];
    ORDER
        .into_iter()
        .filter_map(|engine| {
            let mine: Vec<&&TaskMetrics> = tasks
                .iter()
                .filter(|task| task.attributes.engine == engine)
                .collect();
            (!mine.is_empty()).then(|| EngineRollup {
                engine,
                tasks: mine.len() as i64,
                merged: mine.iter().filter(|task| task.merged_at.is_some()).count() as i64,
            })
        })
        .collect()
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
        let metrics = swarm_metrics(vec![clean, edited], FlagLimits::default());
        assert_eq!(metrics.tasks[0].merged_without_edits, Some(true));
        assert_eq!(metrics.tasks[0].time_to_ready_secs, Some(600));
        assert_eq!(metrics.tasks[0].time_to_merge_secs, Some(900));
        assert_eq!(metrics.tasks[1].merged_without_edits, Some(false));
        let repo = &metrics.repos[0];
        assert_eq!(
            (repo.tasks, repo.merged, repo.merged_without_edits),
            (2, 2, 1)
        );
        assert_eq!(repo.mean_rounds, 1.5);
        assert_eq!(repo.cost_usd_micros_per_merged, Some(300));
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
        let m = swarm_metrics(vec![many, growing, shrinking, merged, stale], limits);
        assert_eq!(m.tasks[0].flags, vec![TaskFlag::NonConverging]);
        assert_eq!(m.tasks[1].flags, vec![TaskFlag::NonConverging]);
        assert!(m.tasks[2].flags.is_empty());
        assert!(m.tasks[3].flags.is_empty(), "a merged task is done");
        assert_eq!(m.tasks[4].flags, vec![TaskFlag::StaleBase]);
        assert_eq!(m.repos[0].flagged, 3);
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
