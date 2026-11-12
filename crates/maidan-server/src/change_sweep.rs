//! The change sweep: an agent's change task closes when its pull request
//! merges.
//!
//! A local stack receives no GitHub webhooks, so a merge was noticed only when
//! someone read the swarm metrics, and the merged change's thread stayed open
//! until someone closed it by hand, holding back every task that depended on
//! it. Every `MAIDAN_CHANGE_SWEEP_SECS` (120 s by default; `0` turns it off)
//! the sweep visits each workspace with a tracked agent pull request. It
//! reads from GitHub the ones not yet seen closed, at most
//! [`REFRESH_LIMIT`] a workspace, the longest unread first, and records each
//! close. Then it settles each recorded close once
//! (`Store::settle_change_pull`): a merge closes the thread through the
//! store's transition path, with `ThreadLanded` in the same transaction, so
//! dependents become claimable and the board moves; a close without a merge
//! leaves the thread open with one notice, for a person or the planner to
//! decide. A merge whose landing the thread's gates refuse (a review not yet
//! approved, the land gate not passed) is said once and tried again each
//! sweep, at most [`REFRESH_LIMIT`] closes a workspace, never-tried closes
//! first: a gate lifts later, and a refusal settled for good left the merged
//! work open, and its dependents waiting, until someone closed it by hand.
//! A thread closed or cancelled in the meantime settles without landing.
//!
//! A thread that only watches a pull request (`watch_pull`) closes when it
//! merges and when it closes without merging, with a message saying which: it
//! has no result or delivery of its own, so no land or review gate holds it,
//! and nobody is left to decide anything.
//!
//! A merge also tells the other workers in its repository: each live claim
//! in the same workspace whose footprint covers a path the merged pull
//! request touched gets one `rebase_needed` notice
//! (`Store::notify_change_merges`), a `ThreadNotice` event and a message,
//! so its worker rebases before it delivers rather than finding out in
//! review. A round delivered on a base `MAIDAN_STALE_BASE_COMMITS` or more
//! behind gets a `stale_base` notice from the egress worker
//! ([`post_notice`] is shared).
//!
//! A merge is then checked for whether it held ([`crate::change_outcomes`]):
//! reverts, follow-up fixes and human effort, in one batch per workspace an
//! hour.
//!
//! An unattended task that stops converging (still open after the round
//! limit, or a diff that grew each round) is handed to a session once
//! ([`hand_off_stuck`]): its claim ends, its engine becomes `session`, a
//! `ThreadHandoff` event and a thread message say so, and its Slack thread
//! gets a line. Where the workspace says claims on Linear, its issue gets the
//! handoff packet (rounds, cost, open findings, the pull request and its
//! head, the models used) and then the release.
//!
//! The same read carries the checks on each open pull request's head: the
//! sweep records them on its thread and says what changed
//! ([`crate::ci_watch`]). A token that may not read the checks in GraphQL
//! (a fine-grained one never can) reads them from the Actions and
//! commit-status APIs instead, several REST calls a pull request. With the
//! GitHub ingress configured (`MAIDAN_GITHUB_WEBHOOK_SECRET`), GitHub's
//! `workflow_run`, `check_suite`, `check_run` and `status` events say when
//! they change, so the sweep spends those calls on a pull request only when
//! an event arrived since its last read, its head moved, or that read is
//! older than `MAIDAN_CHANGE_CI_BACKSTOP_SECS` ([`actions_read`]). Without
//! the ingress no event would ever arrive, so it reads them every pass.
//!
//! Where a workspace has turned Linear claim comments on, the sweep also
//! says a task's claim, and its release when it ends without merging, on the
//! Linear issue its attributes name ([`post_linear_notices`]).
//!
//! The swarm-metrics read shares the refresh but only records closes: it runs
//! inside a reader's request, and landing a thread there would attribute the
//! close to whoever looked. The sweep runs in no request, so the landing is
//! the system's.

use std::time::Duration;

use maidan_types::{
    ChangeSettlement, MemberId, MessageOrigin, MessageProvenance, PostedNotice, ThreadId,
    WorkspaceId,
};

use crate::github::{ActionsRead, ChecksUnread, GithubError, GithubPullSnapshot};
use crate::state::AppState;

/// Default seconds between sweeps.
pub const DEFAULT_SWEEP_SECS: u64 = 120;

/// Default commits behind at which a delivered round's base is stale; the
/// same line the swarm metrics flag at.
pub const DEFAULT_STALE_BASE_COMMITS: i64 = 50;

/// Tracked pull requests read from GitHub, and closes settled, per workspace
/// per pass: bounded so neither a tick nor a metrics read fans out into
/// hundreds of GitHub calls. The rest go on the next pass.
pub const REFRESH_LIMIT: i64 = 25;

/// Workspaces visited per tick.
const WORKSPACES_PER_TICK: i64 = 1000;

/// Default seconds after which the sweep reads a pull request's CI through
/// the Actions API again though no CI event said it changed: a backstop for
/// a delivery GitHub or the forwarder dropped.
pub const DEFAULT_CI_BACKSTOP_SECS: u64 = 1800;

/// What one sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepStats {
    /// Threads closed because their pull request merged, watches included.
    pub landed: usize,
    /// Watch threads closed because their pull request closed without
    /// merging.
    pub watches_closed_unmerged: usize,
    /// Notices posted: a close without a merge, or a merge whose thread's
    /// close was refused.
    pub noticed: usize,
    /// `rebase_needed` notices given to other claims a merge overlapped.
    pub rebase_notices: usize,
    /// Claim and release comments queued for tasks' Linear issues.
    pub linear_notices: usize,
    /// Non-converging unattended tasks handed to a session.
    pub handed_off: usize,
    /// CI notices posted on tracked pull requests' threads (`pr_opened`,
    /// `ci_failed`, `checks_done`).
    pub ci_notices: usize,
}

/// Read `MAIDAN_CHANGE_SWEEP_SECS`. An unset or unparsable value keeps the
/// 120 s default and `0` turns the sweep off (`None`).
pub fn config_from_env() -> Option<Duration> {
    interval_from_raw(std::env::var("MAIDAN_CHANGE_SWEEP_SECS").ok())
}

/// Read `MAIDAN_STALE_BASE_COMMITS`. An unset, unparsable or non-positive
/// value keeps the default of 50.
pub fn stale_base_commits_from_env() -> i64 {
    stale_base_from_raw(std::env::var("MAIDAN_STALE_BASE_COMMITS").ok())
}

fn stale_base_from_raw(raw: Option<String>) -> i64 {
    raw.and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|commits| *commits > 0)
        .unwrap_or(DEFAULT_STALE_BASE_COMMITS)
}

/// Read `MAIDAN_CHANGE_CI_BACKSTOP_SECS`. An unset or unparsable value keeps
/// the 1800 s default; `0` reads CI on every pass, as without the ingress.
pub fn ci_backstop_from_env() -> Duration {
    ci_backstop_from_raw(std::env::var("MAIDAN_CHANGE_CI_BACKSTOP_SECS").ok())
}

fn ci_backstop_from_raw(raw: Option<String>) -> Duration {
    Duration::from_secs(
        raw.and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_CI_BACKSTOP_SECS),
    )
}

/// Whether this pass reads a tracked pull request's CI through the Actions
/// API, given what the store knows (`marks`). Without a recorded read, or
/// with a CI event at or after the start of the last one, or a last read
/// older than `backstop`: yes. Otherwise only if the head moved since.
pub fn actions_read(
    marks: Option<&maidan_types::ChangeCiMarks>,
    now: chrono::DateTime<chrono::Utc>,
    backstop: Duration,
) -> ActionsRead {
    let Some(marks) = marks else {
        return ActionsRead::Read;
    };
    let (Some(read_at), Some(read_head)) = (marks.read_at, marks.read_head.as_ref()) else {
        return ActionsRead::Read;
    };
    // `>=`: the read is stamped with when it began, so an event that
    // arrived while it ran, or in the same instant, is read again.
    if marks.event_at.is_some_and(|event| event >= read_at) {
        return ActionsRead::Read;
    }
    // A read stamped in the future (a clock step) is not yet due.
    if (now - read_at).to_std().is_ok_and(|age| age >= backstop) {
        return ActionsRead::Read;
    }
    ActionsRead::IfHeadMoved(read_head.clone())
}

fn interval_from_raw(raw: Option<String>) -> Option<Duration> {
    let secs = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_SWEEP_SECS);
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// Record the close of each tracked pull request in `workspace_id` that
/// GitHub now reports closed, reading at most [`REFRESH_LIMIT`]. Best effort:
/// a failed read leaves it open until a later pass, behind the ones read
/// longer ago.
pub async fn refresh_open_pulls(state: &AppState, workspace_id: WorkspaceId) {
    if let Some(sender) = state.github_sender.as_deref() {
        refresh_open_pulls_with(state.store.as_ref(), sender, workspace_id).await;
    }
}

/// [`refresh_open_pulls`] given its two parts rather than the whole state, so
/// the MCP server, which holds neither the state nor the sender, can be handed
/// it ([`McpPullRefresher`]).
pub async fn refresh_open_pulls_with(
    store: &dyn maidan_store::Store,
    sender: &dyn crate::github::GithubSender,
    workspace_id: WorkspaceId,
) {
    refresh(store, sender, workspace_id, None).await;
}

/// [`refresh_open_pulls`], and with `stats` (the sweep's, which runs in no
/// request) the checks each open pull request's read carried are recorded
/// too, with the notices they owe ([`crate::ci_watch`]). A metrics read
/// passes none: notices said inside a reader's request would be the
/// reader's.
async fn refresh(
    store: &dyn maidan_store::Store,
    sender: &dyn crate::github::GithubSender,
    workspace_id: WorkspaceId,
    mut sweep: Option<(&AppState, &mut SweepStats)>,
) {
    let Some(git) = sender.git() else {
        return;
    };
    let open = match store
        .list_open_change_pulls(workspace_id, REFRESH_LIMIT)
        .await
    {
        Ok(open) => open,
        Err(err) => {
            tracing::warn!(error = %err, "change sweep: open pull requests not read");
            return;
        }
    };
    // Per repository, what this pass learned about the token's access to
    // its checks: the access is per repository, and one read that comes back
    // readable while another is refused must not resolve and raise the
    // alert in one pass, so a refusal outranks a read.
    let mut checks_seen: std::collections::BTreeMap<String, RepoChecks> =
        std::collections::BTreeMap::new();
    // CI events arrive only through the ingress; without it, waiting for one
    // would leave CI notices to the backstop alone.
    let event_driven = sweep
        .as_ref()
        .is_some_and(|(state, _)| state.github.is_some());
    let backstop = ci_backstop_from_env();
    for (repo, pull_number) in open {
        // Stamped whatever the read says, so a pull request GitHub keeps
        // failing on cannot hold the front of the queue.
        if let Err(err) = store
            .mark_change_pull_checked(workspace_id, &repo, pull_number, chrono::Utc::now())
            .await
        {
            tracing::warn!(repo, pull_number, error = %err, "change sweep: check time not recorded");
        }
        let actions = if sweep.is_none() {
            // A metrics read records no checks, so the Actions calls would
            // buy it nothing.
            ActionsRead::Skip
        } else if !event_driven {
            ActionsRead::Read
        } else {
            match store
                .change_pull_ci_marks(workspace_id, &repo, pull_number)
                .await
            {
                Ok(marks) => actions_read(marks.as_ref(), chrono::Utc::now(), backstop),
                Err(err) => {
                    tracing::warn!(repo, pull_number, error = %err, "change sweep: CI marks not read; reading CI");
                    ActionsRead::Read
                }
            }
        };
        // Taken before the read, so an event that arrives while it runs is
        // later than it and the next pass reads again.
        let read_began = chrono::Utc::now();
        // One read a tick serves the close, the ready label and the checks.
        let snapshot = match git.pull_snapshot(&repo, pull_number, actions).await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                tracing::debug!(repo, pull_number, error = %err, "change sweep: pull request not refreshed");
                continue;
            }
        };
        let seen = RepoChecks::from_snapshot(&snapshot);
        if let RepoChecks::Denied { endpoint, detail } = &seen {
            tracing::warn!(
                repo,
                pull_number,
                endpoint = %endpoint,
                detail = %detail,
                "change sweep: GitHub refused this token the pull request's checks, and its Actions runs or commit statuses too; no CI notice can fire"
            );
        }
        let held = checks_seen
            .entry(repo.clone())
            .or_insert(RepoChecks::Unknown);
        if seen.rank() > held.rank() {
            *held = seen;
        }
        let brief = &snapshot.brief;
        if brief.open {
            if let Some((state, stats)) = sweep.as_mut() {
                stats.ci_notices += crate::ci_watch::record_checks(
                    state,
                    git,
                    workspace_id,
                    &repo,
                    pull_number,
                    &snapshot,
                )
                .await;
                // Only a read that answered counts: one that carried no
                // checks (skipped, or deferred by a rate limit) leaves the
                // next pass due, so nothing an event said is lost.
                let answered = snapshot.checks.is_some()
                    || matches!(snapshot.checks_unread, Some(ChecksUnread::Denied { .. }));
                if answered
                    && let Err(err) = store
                        .mark_change_pull_ci_read(
                            workspace_id,
                            &repo,
                            pull_number,
                            &brief.head_sha,
                            read_began,
                        )
                        .await
                {
                    tracing::warn!(repo, pull_number, error = %err, "change sweep: CI read not recorded");
                }
            }
            // A session's pull request is ready when it carries the label;
            // the first sighting stamps it (Maidan's own flip stamps the
            // change flow's).
            if brief
                .labels
                .iter()
                .any(|label| label == maidan_types::READY_LABEL)
                && let Err(err) = store
                    .mark_change_pull_ready(workspace_id, &repo, pull_number, chrono::Utc::now())
                    .await
            {
                tracing::warn!(repo, pull_number, error = %err, "change sweep: ready not recorded");
            }
            continue;
        }
        let closed_at = brief
            .closed_at
            .or(brief.merged_at)
            .unwrap_or_else(chrono::Utc::now);
        // A stack tool may land a pull request's head itself and then close
        // it rather than merge it. Closed counts as landed only when its head
        // was ahead of the base while open (a branch reset to its base and
        // closed lands nothing) and is on the base now; any other close is
        // not a landing. Asked once: a recorded close is not read again.
        let was_ahead = brief.merged_at.is_none()
            && store
                .pull_head_was_ahead(workspace_id, &repo, pull_number, &brief.head_sha)
                .await
                .unwrap_or_else(|err| {
                    tracing::warn!(repo, pull_number, error = %err, "change sweep: head not looked up");
                    false
                });
        let landed_head = match brief.merged_at {
            Some(_) => Some(brief.head_sha.clone()),
            None if !was_ahead => None,
            None => match closed_landing(git, &repo, brief, pull_number, closed_at).await {
                Ok(landed) => landed,
                // A head GitHub no longer has, or a ref it refuses, cannot be
                // on the base.
                Err(err) if err.is_not_found() || matches!(err, GithubError::Refused(_)) => None,
                // Anything else may pass: the close waits for the next tick
                // rather than be recorded unmerged on a failed read.
                Err(err) => {
                    tracing::debug!(repo, pull_number, error = %err, "change sweep: closed pull request not compared with its base");
                    continue;
                }
            },
        };
        let merged_at = brief
            .merged_at
            .or_else(|| landed_head.as_ref().map(|_| closed_at));
        let merged_head = landed_head.as_deref();
        if let Err(err) = store
            .close_change_pull(
                workspace_id,
                &repo,
                pull_number,
                merged_at,
                merged_head,
                closed_at,
            )
            .await
        {
            tracing::warn!(repo, pull_number, error = %err, "change sweep: close not recorded");
        }
    }
    alert_checks_denied(checks_seen).await;
}

/// What one pass learned about whether GitHub shows the token a
/// repository's checks.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RepoChecks {
    /// No read this pass said: each failed for now, or carried no checks.
    Unknown,
    /// A pull request's checks read.
    Read,
    /// GitHub refused a pull request's checks as a matter of access:
    /// `endpoint` is the call that refused, `detail` what it answered.
    Denied { endpoint: String, detail: String },
}

impl RepoChecks {
    fn from_snapshot(snapshot: &GithubPullSnapshot) -> Self {
        match &snapshot.checks_unread {
            Some(ChecksUnread::Denied { endpoint, detail }) => Self::Denied {
                endpoint: endpoint.clone(),
                detail: detail.clone(),
            },
            Some(ChecksUnread::Deferred) => Self::Unknown,
            None if snapshot.checks.is_some() => Self::Read,
            None => Self::Unknown,
        }
    }

    /// Which of two answers in one pass stands: a refusal, then a read.
    fn rank(&self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Read => 1,
            Self::Denied { .. } => 2,
        }
    }
}

/// Sweeps in a row, and minutes, a repository's checks must stay refused
/// before the operator hears of it. A refusal that clears inside that is
/// not one an operator can act on: on 2026-11-09 the alert fired and
/// resolved twice in 20 minutes on a token that had every permission.
const DENIED_PASSES: u32 = 3;
const DENIED_FOR: Duration = Duration::from_secs(15 * 60);

/// When each repository's run of refused sweeps began and how many it has
/// lasted. In memory: a fresh process starts the count again, which only
/// delays the alert for a refusal that persists.
static DENIED_STREAKS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, u32)>>,
> = std::sync::OnceLock::new();

/// What a repository's alert does after a pass.
#[derive(Debug, PartialEq, Eq)]
enum AlertStep {
    Raise,
    Resolve,
    Hold,
}

/// Count `seen` into `repo`'s streak at `now`. A read ends the streak and
/// resolves; a pass that learned nothing leaves both as they stand; a
/// refusal raises once it has lasted [`DENIED_PASSES`] sweeps and
/// [`DENIED_FOR`].
fn next_step(
    streaks: &mut std::collections::HashMap<String, (std::time::Instant, u32)>,
    repo: &str,
    seen: &RepoChecks,
    now: std::time::Instant,
) -> AlertStep {
    match seen {
        RepoChecks::Unknown => AlertStep::Hold,
        RepoChecks::Read => {
            streaks.remove(repo);
            AlertStep::Resolve
        }
        RepoChecks::Denied { .. } => {
            let (since, passes) = streaks.entry(repo.to_string()).or_insert((now, 0));
            *passes += 1;
            if *passes >= DENIED_PASSES && now.duration_since(*since) >= DENIED_FOR {
                AlertStep::Raise
            } else {
                AlertStep::Hold
            }
        }
    }
}

/// Raise the operator alert for each repository whose checks GitHub has
/// refused for long enough, and resolve it for each whose checks read. It
/// is a condition, not an event: it posts once per outage, however many
/// passes and restarts it lasts, and once more when it clears. A repository
/// no read this pass spoke for is left as it stands.
async fn alert_checks_denied(checks_seen: std::collections::BTreeMap<String, RepoChecks>) {
    let now = std::time::Instant::now();
    for (repo, seen) in checks_seen {
        let step = {
            let mut streaks = DENIED_STREAKS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            next_step(&mut streaks, &repo, &seen, now)
        };
        let key = format!("github-checks-denied:{repo}");
        match (step, &seen) {
            (AlertStep::Raise, RepoChecks::Denied { endpoint, detail }) => {
                crate::ops_alert::raise(
                    &key,
                    &format!(
                        "GitHub has refused Maidan's token the CI on {repo}'s pull requests on every sweep for at least {} minutes, so no ci_failed or checks_done notice can fire. The refusing call is {endpoint}, which answered: {detail}",
                        DENIED_FOR.as_secs() / 60
                    ),
                )
                .await;
            }
            (AlertStep::Resolve, _) => {
                crate::ops_alert::resolve(
                    &key,
                    &format!(
                        "Resolved: Maidan's token reads the CI on {repo}'s pull requests again, so ci_failed and checks_done notices fire."
                    ),
                )
                .await;
            }
            _ => {}
        }
    }
}

/// The commit a closed, unmerged pull request landed as on its base, if it
/// did. A stack tool lands the head itself, so the head is on the base. A
/// merge queue that squashes into a commit of its own and then closes the
/// pull request (Graphite's does) leaves the head off the base; that landing
/// is the base commit, from the day before the close, whose subject ends
/// `(#N)`. A revert's subject ends in a quote, so it is not one.
async fn closed_landing(
    git: &dyn crate::github::GithubGit,
    repo: &str,
    brief: &crate::github::GithubPullBrief,
    pull_number: i64,
    closed_at: chrono::DateTime<chrono::Utc>,
) -> Result<Option<String>, GithubError> {
    if git
        .commit_on_branch(repo, &brief.base, &brief.head_sha)
        .await?
    {
        return Ok(Some(brief.head_sha.clone()));
    }
    let suffix = format!("(#{pull_number})");
    Ok(git
        .branch_commits_since(repo, &brief.base, closed_at - chrono::Duration::days(1))
        .await?
        .into_iter()
        .find(|commit| {
            commit
                .message
                .lines()
                .next()
                .is_some_and(|subject| subject.trim_end().ends_with(&suffix))
        })
        .map(|commit| commit.sha))
}

/// The swarm-metrics refresh, handed to the MCP server so its
/// `get_swarm_metrics` reads GitHub the way the route does.
pub struct McpPullRefresher {
    pub store: std::sync::Arc<dyn maidan_store::Store>,
    pub sender: std::sync::Arc<dyn crate::github::GithubSender>,
}

#[async_trait::async_trait]
impl maidan_mcp::OpenPullRefresher for McpPullRefresher {
    async fn refresh_open_pulls(&self, workspace_id: WorkspaceId) {
        refresh_open_pulls_with(self.store.as_ref(), self.sender.as_ref(), workspace_id).await;
    }
}

/// One pass over every workspace with a tracked pull request still to read
/// or settle.
pub async fn sweep_once(state: &AppState) -> SweepStats {
    let mut stats = SweepStats::default();
    let workspaces = match state
        .store
        .list_change_sweep_workspaces(WORKSPACES_PER_TICK)
        .await
    {
        Ok(workspaces) => workspaces,
        Err(err) => {
            tracing::warn!(error = %err, "change sweep: workspaces not read");
            return stats;
        }
    };
    for workspace_id in workspaces {
        if crate::suspension::held_back(state, workspace_id).await {
            continue;
        }
        if let Some(sender) = state.github_sender.as_deref() {
            refresh(
                state.store.as_ref(),
                sender,
                workspace_id,
                Some((state, &mut stats)),
            )
            .await;
        }
        settle_closed_pulls(state, workspace_id, &mut stats).await;
        notify_merges(state, workspace_id, &mut stats).await;
        hand_off_stuck(state, workspace_id, &mut stats).await;
        crate::change_outcomes::check_outcomes(state, workspace_id).await;
    }
    stats.linear_notices = post_linear_notices(state).await;
    stats
}

/// Queue one notice on its Linear issue, recorded once. The issue is looked
/// up first, because the allowlist blesses its team: an issue in a team the
/// workspace has not blessed, or one that is not found, is recorded as
/// skipped and never written to. `None` when the lookup failed otherwise and
/// the notice should be tried again; `Some(queued)` once recorded.
async fn queue_linear_notice(
    state: &AppState,
    sender: &dyn crate::linear::LinearSender,
    item: &maidan_types::LinearNoticeDue,
) -> Option<bool> {
    let (outcome, send) = match sender.issue(&item.issue).await {
        Ok(found) => {
            let target = maidan_types::EgressTarget::Linear {
                team_id: found.team_id.clone(),
                issue: found.identifier.clone(),
            };
            match state
                .store
                .is_egress_target_allowed(
                    item.workspace_id,
                    maidan_types::EgressSurface::Linear,
                    &target.allowlist_selector(),
                )
                .await
            {
                Ok(true) => (
                    "queued".to_string(),
                    Some(maidan_types::NewEgressOutbox {
                        workspace_id: item.workspace_id,
                        thread_id: item.thread_id,
                        source_log_id: item.outbox_key(),
                        target,
                        body: item.body(),
                        kind: maidan_types::EgressKind::Notice,
                    }),
                ),
                Ok(false) => (
                    format!(
                        "skipped: team {} is not in the workspace egress allowlist",
                        found.team_id
                    ),
                    None,
                ),
                Err(err) => {
                    tracing::warn!(error = %err, "linear notices: allowlist not read");
                    return None;
                }
            }
        }
        Err(crate::linear::LinearError::NotFound(_)) => {
            ("skipped: the issue was not found".to_string(), None)
        }
        Err(err) => {
            tracing::warn!(issue = item.issue, error = %err, "linear notices: issue not read; tried again next tick");
            return None;
        }
    };
    let sent = send.is_some();
    match state.store.record_linear_notice(item, &outcome, send).await {
        Ok(recorded) => Some(recorded && sent),
        Err(err) => {
            tracing::warn!(issue = item.issue, error = %err, "linear notices: not recorded");
            None
        }
    }
}

/// Hand each non-converging unattended task in `workspace_id` to a session,
/// once.
/// Best effort past the handoff itself: the packet, the message and the
/// Slack line are said after it commits, and a failed one is logged.
async fn hand_off_stuck(state: &AppState, workspace_id: WorkspaceId, stats: &mut SweepStats) {
    let pulls = match state
        .store
        .list_change_pulls(workspace_id, None, None, None, None, None)
        .await
    {
        Ok(pulls) => pulls,
        Err(err) => {
            tracing::warn!(error = %err, "handoff: pull requests not read");
            return;
        }
    };
    let metrics = maidan_types::swarm_metrics(
        pulls,
        maidan_types::FlagLimits::default(),
        chrono::Utc::now(),
    );
    for task in metrics.tasks {
        if task.handed_off
            || task.attributes.engine != Some(maidan_types::TaskEngine::Unattended)
            || !task.flags.contains(&maidan_types::TaskFlag::NonConverging)
        {
            continue;
        }
        let rounds = task.rounds.len() as i64;
        let handed = match state
            .store
            .hand_off_thread(task.thread_id, &task.repo, task.pull_number, rounds)
            .await
        {
            Ok(Some(handed)) => handed,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(repo = task.repo, pull_number = task.pull_number, error = %err, "handoff: not recorded");
                continue;
            }
        };
        stats.handed_off += 1;
        // The notices' own key space (negative, fixed per task and kind), so
        // the Slack line never meets the result's delivery row in the
        // outbox's dedup.
        let slack_key = maidan_types::LinearNoticeDue {
            workspace_id,
            thread_id: task.thread_id,
            kind: maidan_types::LinearNoticeKind::Handoff,
            issue: String::new(),
            title: None,
            engine: None,
            holder: None,
            reason: None,
        }
        .outbox_key();
        crate::routes::publish_stored(state, handed.event.clone()).await;
        let result = state
            .store
            .get_thread_result(task.thread_id)
            .await
            .ok()
            .flatten()
            .map(|stored| stored.result);
        let line = format!(
            "Handed to a session: {}#{} did not converge in {rounds} rounds, so its claim ended and no unattended worker will take it again.",
            task.repo, task.pull_number
        );
        if let Some(author_id) = handed.author_id {
            let new = maidan_types::NewMessage {
                thread_id: task.thread_id,
                author_id,
                body: line.clone(),
                metadata: serde_json::json!({
                    "github": { "repo": task.repo, "issue": task.pull_number },
                    "handoff": { "rounds": rounds },
                }),
                content: None,
            };
            // The GitHub origin keeps egress from posting the line onto the
            // pull request, as for the change notices.
            let posted = state
                .store
                .post_bridged_message_with_event(
                    new,
                    MessageProvenance::reported(MessageOrigin::Github),
                )
                .await;
            match posted {
                Ok(Some((_, stored))) => crate::routes::publish_stored(state, stored).await,
                Ok(None) => {}
                Err(err) => tracing::warn!(error = %err, "handoff: message not posted"),
            }
        }
        for target in result
            .as_ref()
            .and_then(maidan_types::parse_waiter_result)
            .map(|waiter| waiter.deliver_to)
            .unwrap_or_default()
        {
            let Some(target @ maidan_types::EgressTarget::Slack { .. }) = target.to_egress_target()
            else {
                continue;
            };
            let queued = state
                .store
                .enqueue_egress(maidan_types::NewEgressOutbox {
                    workspace_id,
                    thread_id: task.thread_id,
                    source_log_id: slack_key,
                    target,
                    body: line.clone(),
                    kind: maidan_types::EgressKind::Notice,
                })
                .await;
            if let Err(err) = queued {
                tracing::warn!(error = %err, "handoff: slack line not queued");
            }
        }
        let issue = task
            .attributes
            .linear_issue
            .clone()
            .or_else(|| task.linear_issue.clone());
        let (Some(issue), Some(sender)) = (issue, state.linear_sender.as_ref()) else {
            continue;
        };
        if !matches!(
            state.store.linear_claim_comments(workspace_id).await,
            Ok(true)
        ) {
            continue;
        }
        let packet = maidan_types::LinearNoticeDue {
            workspace_id,
            thread_id: task.thread_id,
            kind: maidan_types::LinearNoticeKind::Handoff,
            issue,
            title: None,
            engine: Some(maidan_types::TaskEngine::Unattended),
            holder: None,
            reason: Some(handoff_packet(&task, result.as_ref())),
        };
        queue_linear_notice(state, sender.as_ref(), &packet).await;
    }
}

/// The handoff packet a session picks the task up from: why, the pull
/// request and its head, each round, the cost, the open findings and the
/// models the result reported.
pub fn handoff_packet(
    task: &maidan_types::TaskMetrics,
    result: Option<&serde_json::Value>,
) -> String {
    let mut lines = vec![format!(
        "Handed to a session after {} rounds without converging.",
        task.rounds.len()
    )];
    let head = task
        .rounds
        .last()
        .map(|round| format!(" at {}", round.commit_sha))
        .unwrap_or_default();
    lines.push(format!(
        "PR: https://github.com/{}/pull/{}{head}",
        task.repo, task.pull_number
    ));
    lines.push("Rounds:".into());
    for round in &task.rounds {
        let base = match round.base_behind {
            Some(behind) => format!("base {behind} behind"),
            None => "base lag unknown".into(),
        };
        let rebase = if round.rebase {
            ", a squash-rebase"
        } else {
            ""
        };
        lines.push(format!(
            "- {}: {} lines, {base}{rebase}",
            round.round, round.diff_lines
        ));
    }
    lines.push(format!(
        "Cost: ${:.2}",
        task.cost_usd_micros as f64 / 1_000_000.0
    ));
    let findings = result
        .and_then(maidan_types::parse_waiter_result)
        .map(|waiter| waiter.findings)
        .unwrap_or_default();
    if findings.is_empty() {
        lines.push("Open findings: none reported".into());
    } else {
        lines.push(format!("Open findings: {}", findings.len()));
        for finding in findings.iter().take(5) {
            let first = finding.body.lines().next().unwrap_or_default();
            lines.push(format!("- {}: {first}", finding.file));
        }
    }
    let models: Vec<String> = ["model", "author_model", "red_team_model"]
        .into_iter()
        .filter_map(|key| {
            result
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_str)
                .map(|model| format!("{key} {model}"))
        })
        .collect();
    lines.push(if models.is_empty() {
        "Models: not reported".into()
    } else {
        format!("Models: {}", models.join(", "))
    });
    lines.join("\n")
}

/// Queue the claim and release comments due in every workspace that turned
/// them on ([`queue_linear_notice`]).
pub async fn post_linear_notices(state: &AppState) -> usize {
    let Some(sender) = state.linear_sender.as_ref() else {
        return 0;
    };
    let workspaces = match state
        .store
        .list_linear_claim_comment_workspaces(WORKSPACES_PER_TICK)
        .await
    {
        Ok(workspaces) => workspaces,
        Err(err) => {
            tracing::warn!(error = %err, "linear notices: workspaces not read");
            return 0;
        }
    };
    let mut queued = 0;
    for workspace_id in workspaces {
        if crate::suspension::held_back(state, workspace_id).await {
            continue;
        }
        let due = match state
            .store
            .list_linear_notices_due(workspace_id, REFRESH_LIMIT)
            .await
        {
            Ok(due) => due,
            Err(err) => {
                tracing::warn!(error = %err, "linear notices: due notices not read");
                continue;
            }
        };
        for item in due {
            if queue_linear_notice(state, sender.as_ref(), &item).await == Some(true) {
                queued += 1;
            }
        }
    }
    queued
}

async fn notify_merges(state: &AppState, workspace_id: WorkspaceId, stats: &mut SweepStats) {
    let posted = match state
        .store
        .notify_change_merges(workspace_id, REFRESH_LIMIT)
        .await
    {
        Ok(posted) => posted,
        Err(err) => {
            tracing::warn!(error = %err, "change sweep: merges not notified");
            return;
        }
    };
    for notice in posted {
        post_notice(state, notice).await;
        stats.rebase_notices += 1;
    }
}

/// Publish a recorded change notice's event and post the message that says
/// it. Best effort: the notice is already recorded, and is not said twice.
pub async fn post_notice(state: &AppState, posted: PostedNotice) {
    crate::routes::publish_stored(state, posted.event).await;
    let Some(author_id) = posted.author_id else {
        return;
    };
    let notice = &posted.notice;
    let new = maidan_types::NewMessage {
        thread_id: posted.thread_id,
        author_id,
        body: notice.sentence(),
        metadata: serde_json::json!({
            "github": { "repo": notice.repo, "issue": notice.pull_number },
            "change_notice": { "kind": notice.kind.as_str() },
        }),
        content: None,
    };
    // The GitHub origin keeps egress from posting the notice onto the pull
    // request it is about, as for the close notices.
    let posted = state
        .store
        .post_bridged_message_with_event(new, MessageProvenance::reported(MessageOrigin::Github))
        .await;
    match posted {
        Ok(Some((_, stored))) => crate::routes::publish_stored(state, stored).await,
        Ok(None) => {}
        Err(err) => tracing::warn!(
            repo = notice.repo,
            pull_number = notice.pull_number,
            error = %err,
            "change notice: message not posted"
        ),
    }
}

async fn settle_closed_pulls(state: &AppState, workspace_id: WorkspaceId, stats: &mut SweepStats) {
    let closed = match state
        .store
        .list_unsettled_change_pulls(workspace_id, REFRESH_LIMIT)
        .await
    {
        Ok(closed) => closed,
        Err(err) => {
            tracing::warn!(error = %err, "change sweep: closed pull requests not read");
            return;
        }
    };
    for (repo, pull_number) in closed {
        match state
            .store
            .settle_change_pull(workspace_id, &repo, pull_number)
            .await
        {
            Ok(ChangeSettlement::Landed(stored)) => {
                for event in stored {
                    crate::routes::publish_stored(state, event).await;
                }
                stats.landed += 1;
            }
            Ok(ChangeSettlement::WatchClosed { merged, events }) => {
                for event in events {
                    crate::routes::publish_stored(state, event).await;
                }
                if merged {
                    stats.landed += 1;
                } else {
                    stats.watches_closed_unmerged += 1;
                }
            }
            Ok(ChangeSettlement::ClosedUnmerged {
                thread_id,
                author_id,
            }) => {
                let notice = Notice {
                    thread_id,
                    author_id,
                    repo: &repo,
                    pull_number,
                    merged: false,
                };
                notice
                    .post(state, format!("#{pull_number} closed without merging"))
                    .await;
                stats.noticed += 1;
            }
            Ok(ChangeSettlement::CloseRefused {
                thread_id,
                author_id,
                reason,
            }) => {
                let notice = Notice {
                    thread_id,
                    author_id,
                    repo: &repo,
                    pull_number,
                    merged: true,
                };
                notice
                    .post(
                        state,
                        format!(
                            "#{pull_number} merged, but this thread stays open until its gates \
                             pass, and lands on the next sweep after that: {reason}"
                        ),
                    )
                    .await;
                stats.noticed += 1;
            }
            Ok(ChangeSettlement::Unchanged) => {}
            Err(err) => {
                tracing::warn!(repo, pull_number, error = %err, "change sweep: close not settled");
            }
        }
    }
}

struct Notice<'a> {
    thread_id: ThreadId,
    author_id: MemberId,
    repo: &'a str,
    pull_number: i64,
    merged: bool,
}

impl Notice<'_> {
    /// Best effort: the close is already settled, or its refusal recorded, so
    /// a failed post is not retried, rather than risk saying it twice.
    async fn post(&self, state: &AppState, body: String) {
        let new = maidan_types::NewMessage {
            thread_id: self.thread_id,
            author_id: self.author_id,
            body,
            metadata: serde_json::json!({
                "github": {
                    "repo": self.repo,
                    "issue": self.pull_number,
                    "closed_unmerged": !self.merged,
                }
            }),
            content: None,
        };
        // The GitHub origin keeps egress from posting the notice back onto
        // the pull request it is about.
        let posted = state
            .store
            .post_bridged_message_with_event(
                new,
                MessageProvenance::reported(MessageOrigin::Github),
            )
            .await;
        match posted {
            Ok(Some((_, stored))) => crate::routes::publish_stored(state, stored).await,
            Ok(None) => {}
            Err(err) => tracing::warn!(
                repo = self.repo,
                pull_number = self.pull_number,
                error = %err,
                "change sweep: notice not posted"
            ),
        }
    }
}

/// The sweep loop, spawned by `main.rs` unless turned off.
pub async fn run(state: AppState, interval: Duration) {
    tracing::info!(interval_secs = interval.as_secs(), "change sweep started");
    let _watched = crate::ops_alert::start_loop("change_sweep", interval);
    loop {
        match crate::ops_alert::guarded_tick("change_sweep", sweep_once(&state)).await {
            Some(stats) if stats != SweepStats::default() => {
                tracing::info!(?stats, "change sweep tick");
            }
            Some(_) => tracing::debug!("change sweep tick: nothing to do"),
            None => {}
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied() -> RepoChecks {
        RepoChecks::Denied {
            endpoint: "GET /repos/o/r/actions/runs".into(),
            detail:
                "github api error: status 403: Resource not accessible by personal access token"
                    .into(),
        }
    }

    #[test]
    fn a_refusal_that_clears_within_minutes_never_raises_the_alert() {
        let mut streaks = std::collections::HashMap::new();
        let t0 = std::time::Instant::now();
        let minute = Duration::from_secs(60);
        // Refused, read, refused, read, as on 2026-11-09: never a raise.
        for (at, seen) in [
            (0, denied()),
            (7, RepoChecks::Read),
            (13, denied()),
            (15, RepoChecks::Read),
        ] {
            assert_ne!(
                next_step(&mut streaks, "o/r", &seen, t0 + minute * at),
                AlertStep::Raise
            );
        }
        // Three refusals inside two minutes are not long enough either.
        for at in 20..23 {
            assert_eq!(
                next_step(&mut streaks, "o/r", &denied(), t0 + minute * at),
                AlertStep::Hold
            );
        }
    }

    #[test]
    fn a_refusal_that_lasts_three_sweeps_and_fifteen_minutes_raises_until_a_read_resolves() {
        let mut streaks = std::collections::HashMap::new();
        let t0 = std::time::Instant::now();
        let minute = Duration::from_secs(60);
        assert_eq!(
            next_step(&mut streaks, "o/r", &denied(), t0),
            AlertStep::Hold
        );
        assert_eq!(
            next_step(&mut streaks, "o/r", &denied(), t0 + minute * 2),
            AlertStep::Hold
        );
        // A pass that learned nothing (a rate limit) neither counts nor resets.
        assert_eq!(
            next_step(&mut streaks, "o/r", &RepoChecks::Unknown, t0 + minute * 4),
            AlertStep::Hold
        );
        assert_eq!(
            next_step(&mut streaks, "o/r", &denied(), t0 + minute * 14),
            AlertStep::Hold
        );
        assert_eq!(
            next_step(&mut streaks, "o/r", &denied(), t0 + minute * 16),
            AlertStep::Raise
        );
        // Another repository keeps its own count.
        assert_eq!(
            next_step(&mut streaks, "o/other", &denied(), t0 + minute * 16),
            AlertStep::Hold
        );
        assert_eq!(
            next_step(&mut streaks, "o/r", &RepoChecks::Read, t0 + minute * 18),
            AlertStep::Resolve
        );
        // The read ended the streak: the count starts again.
        assert_eq!(
            next_step(&mut streaks, "o/r", &denied(), t0 + minute * 40),
            AlertStep::Hold
        );
    }

    #[test]
    fn a_rate_limited_actions_read_tells_nothing_and_a_refusal_outranks_a_read() {
        let brief = crate::github::GithubPullBrief {
            number: 1,
            head: "feature/agent-x".into(),
            base: "dev".into(),
            draft: false,
            node_id: "PR_1".into(),
            same_repo: true,
            open: true,
            head_sha: "a".repeat(40),
            labels: vec![],
            merged_at: None,
            closed_at: None,
            merge_commit_sha: None,
        };
        let snapshot =
            |checks: Option<Vec<crate::github::GithubCheck>>,
             checks_unread: Option<ChecksUnread>| GithubPullSnapshot {
                brief: brief.clone(),
                checks,
                checks_unread,
                gate: None,
                ahead: None,
            };
        assert_eq!(
            RepoChecks::from_snapshot(&snapshot(None, Some(ChecksUnread::Deferred))),
            RepoChecks::Unknown
        );
        assert_eq!(
            RepoChecks::from_snapshot(&snapshot(Some(vec![]), None)),
            RepoChecks::Read
        );
        assert_eq!(
            RepoChecks::from_snapshot(&snapshot(None, None)),
            RepoChecks::Unknown
        );
        assert!(denied().rank() > RepoChecks::Read.rank());
        assert!(RepoChecks::Read.rank() > RepoChecks::Unknown.rank());
    }

    #[test]
    fn the_ci_backstop_defaults_to_half_an_hour_and_zero_reads_every_pass() {
        assert_eq!(ci_backstop_from_raw(None), Duration::from_secs(1800));
        assert_eq!(
            ci_backstop_from_raw(Some("soon".into())),
            Duration::from_secs(1800)
        );
        assert_eq!(
            ci_backstop_from_raw(Some(" 600 ".into())),
            Duration::from_secs(600)
        );
        let now = chrono::Utc::now();
        let just_read = maidan_types::ChangeCiMarks {
            event_at: None,
            read_at: Some(now),
            read_head: Some("a".repeat(40)),
        };
        assert_eq!(
            actions_read(
                Some(&just_read),
                now,
                ci_backstop_from_raw(Some("0".into()))
            ),
            ActionsRead::Read
        );
    }

    #[test]
    fn actions_are_read_only_after_an_event_a_push_or_the_backstop() {
        let now = chrono::Utc::now();
        let backstop = Duration::from_secs(1800);
        let head = "a".repeat(40);
        let read = |ago_secs: i64, event: Option<i64>| maidan_types::ChangeCiMarks {
            event_at: event.map(|ago| now - chrono::Duration::seconds(ago)),
            read_at: Some(now - chrono::Duration::seconds(ago_secs)),
            read_head: Some(head.clone()),
        };
        // Never read, or tracked nowhere the store knows: read.
        assert_eq!(actions_read(None, now, backstop), ActionsRead::Read);
        assert_eq!(
            actions_read(Some(&maidan_types::ChangeCiMarks::default()), now, backstop),
            ActionsRead::Read
        );
        // Read a minute ago, no event since: only a moved head is read.
        let quiet = actions_read(Some(&read(60, None)), now, backstop);
        assert_eq!(quiet, ActionsRead::IfHeadMoved(head.clone()));
        assert!(!quiet.reads_at(&head));
        assert!(!quiet.reads_at(&head.to_ascii_uppercase()));
        assert!(quiet.reads_at(&"b".repeat(40)));
        // An event before the last read began was what that read saw.
        assert_eq!(
            actions_read(Some(&read(60, Some(90))), now, backstop),
            ActionsRead::IfHeadMoved(head.clone())
        );
        // An event after it began, or in the same instant, is read.
        assert_eq!(
            actions_read(Some(&read(60, Some(30))), now, backstop),
            ActionsRead::Read
        );
        assert_eq!(
            actions_read(Some(&read(60, Some(60))), now, backstop),
            ActionsRead::Read
        );
        // A last read as old as the backstop is read again without an event.
        assert_eq!(
            actions_read(Some(&read(1799, None)), now, backstop),
            ActionsRead::IfHeadMoved(head.clone())
        );
        assert_eq!(
            actions_read(Some(&read(1800, None)), now, backstop),
            ActionsRead::Read
        );
        assert!(ActionsRead::Read.reads_at(&head));
        assert!(!ActionsRead::Skip.reads_at(&head));
    }

    #[test]
    fn the_sweep_is_on_by_default_and_zero_turns_it_off() {
        let default = Some(Duration::from_secs(DEFAULT_SWEEP_SECS));
        assert_eq!(interval_from_raw(None), default);
        assert_eq!(interval_from_raw(Some("soon".into())), default);
        assert_eq!(
            interval_from_raw(Some(" 30 ".into())),
            Some(Duration::from_secs(30))
        );
        assert_eq!(interval_from_raw(Some("0".into())), None);
    }

    #[test]
    fn the_stale_base_line_defaults_to_fifty_and_must_be_positive() {
        assert_eq!(stale_base_from_raw(None), DEFAULT_STALE_BASE_COMMITS);
        assert_eq!(stale_base_from_raw(Some(" 20 ".into())), 20);
        assert_eq!(stale_base_from_raw(Some("0".into())), 50);
        assert_eq!(stale_base_from_raw(Some("-3".into())), 50);
        assert_eq!(stale_base_from_raw(Some("lots".into())), 50);
    }
}
