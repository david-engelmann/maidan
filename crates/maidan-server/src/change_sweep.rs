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
//! decide.
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
//! A Pi task that stops converging (still open after the round limit, or a
//! diff that grew each round) is handed to a session once
//! ([`hand_off_stuck`]): its claim ends, its engine becomes `session`, a
//! `ThreadHandoff` event and a thread message say so, and its Slack thread
//! gets a line. Where the workspace says claims on Linear, its issue gets the
//! handoff packet (rounds, cost, open findings, the pull request and its
//! head, the models used) and then the release.
//!
//! The same read carries the checks on each open pull request's head: the
//! sweep records them on its thread and says what changed
//! ([`crate::ci_watch`]).
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

use maidan_types::{ChangeSettlement, MemberId, PostedNotice, ThreadId, WorkspaceId};

use crate::github::GithubError;
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

/// What one sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepStats {
    /// Threads closed because their pull request merged.
    pub landed: usize,
    /// Notices posted: a close without a merge, or a merge whose thread's
    /// close was refused.
    pub noticed: usize,
    /// `rebase_needed` notices given to other claims a merge overlapped.
    pub rebase_notices: usize,
    /// Claim and release comments queued for tasks' Linear issues.
    pub linear_notices: usize,
    /// Non-converging Pi tasks handed to a session.
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
    refresh(state, workspace_id, None).await;
}

/// [`refresh_open_pulls`], and with `stats` (the sweep's, which runs in no
/// request) the checks each open pull request's read carried are recorded
/// too, with the notices they owe ([`crate::ci_watch`]). A metrics read
/// passes none: notices said inside a reader's request would be the
/// reader's.
async fn refresh(state: &AppState, workspace_id: WorkspaceId, mut stats: Option<&mut SweepStats>) {
    let Some(git) = state.github_sender.as_ref().and_then(|s| s.git()) else {
        return;
    };
    let open = match state
        .store
        .list_open_change_pulls(workspace_id, REFRESH_LIMIT)
        .await
    {
        Ok(open) => open,
        Err(err) => {
            tracing::warn!(error = %err, "change sweep: open pull requests not read");
            return;
        }
    };
    for (repo, pull_number) in open {
        // Stamped whatever the read says, so a pull request GitHub keeps
        // failing on cannot hold the front of the queue.
        if let Err(err) = state
            .store
            .mark_change_pull_checked(workspace_id, &repo, pull_number, chrono::Utc::now())
            .await
        {
            tracing::warn!(repo, pull_number, error = %err, "change sweep: check time not recorded");
        }
        // One read a tick serves the close, the ready label and the checks.
        let snapshot = match git.pull_snapshot(&repo, pull_number).await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                tracing::debug!(repo, pull_number, error = %err, "change sweep: pull request not refreshed");
                continue;
            }
        };
        let brief = &snapshot.brief;
        if brief.open {
            if let Some(stats) = stats.as_deref_mut() {
                stats.ci_notices += crate::ci_watch::record_checks(
                    state,
                    git,
                    workspace_id,
                    &repo,
                    pull_number,
                    &snapshot,
                )
                .await;
            }
            // A session's pull request is ready when it carries the label;
            // the first sighting stamps it (Maidan's own flip stamps the
            // change flow's).
            if brief
                .labels
                .iter()
                .any(|label| label == maidan_types::READY_LABEL)
            {
                if let Err(err) = state
                    .store
                    .mark_change_pull_ready(workspace_id, &repo, pull_number, chrono::Utc::now())
                    .await
                {
                    tracing::warn!(repo, pull_number, error = %err, "change sweep: ready not recorded");
                }
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
            && state
                .store
                .pull_head_was_ahead(workspace_id, &repo, pull_number, &brief.head_sha)
                .await
                .unwrap_or_else(|err| {
                    tracing::warn!(repo, pull_number, error = %err, "change sweep: head not looked up");
                    false
                });
        let merged_at = match brief.merged_at {
            Some(merged_at) => Some(merged_at),
            None if !was_ahead => None,
            None => match git
                .commit_on_branch(&repo, &brief.base, &brief.head_sha)
                .await
            {
                Ok(true) => Some(closed_at),
                Ok(false) => None,
                // A head GitHub no longer has, or a ref it refuses, cannot be
                // on the base.
                Err(err) if err.is_not_found() || matches!(err, GithubError::Refused(_)) => None,
                // Anything else may pass: the close waits for the next tick
                // rather than be recorded unmerged on a failed read.
                Err(err) => {
                    tracing::debug!(repo, pull_number, error = %err, "change sweep: closed head not compared with its base");
                    continue;
                }
            },
        };
        let merged_head = merged_at.map(|_| brief.head_sha.as_str());
        if let Err(err) = state
            .store
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
        refresh(state, workspace_id, Some(&mut stats)).await;
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

/// Hand each non-converging Pi task in `workspace_id` to a session, once.
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
            || task.attributes.engine != Some(maidan_types::TaskEngine::Pi)
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
            "Handed to a session: {}#{} did not converge in {rounds} rounds, so its claim ended and Pi will not take it again.",
            task.repo, task.pull_number
        );
        if let Some(author_id) = handed.author_id {
            let new = maidan_types::NewMessage {
                thread_id: task.thread_id,
                author_id,
                body: line.clone(),
                // `github` keeps egress from posting the line onto the pull
                // request, as for the change notices.
                metadata: serde_json::json!({
                    "github": { "repo": task.repo, "issue": task.pull_number },
                    "handoff": { "rounds": rounds },
                }),
                content: None,
            };
            match state.store.post_message_with_event(new, None).await {
                Ok((_, stored)) => crate::routes::publish_stored(state, stored).await,
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
            engine: Some(maidan_types::TaskEngine::Pi),
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
        // `github` keeps egress from posting the notice onto the pull
        // request it is about, as for the close notices.
        metadata: serde_json::json!({
            "github": { "repo": notice.repo, "issue": notice.pull_number },
            "change_notice": { "kind": notice.kind.as_str() },
        }),
        content: None,
    };
    match state.store.post_message_with_event(new, None).await {
        Ok((_, stored)) => crate::routes::publish_stored(state, stored).await,
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
                        format!("#{pull_number} merged, but this thread stays open: {reason}"),
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
    /// Best effort: the close is already settled, so a failed post is not
    /// retried, rather than risk saying it twice.
    async fn post(&self, state: &AppState, body: String) {
        let new = maidan_types::NewMessage {
            thread_id: self.thread_id,
            author_id: self.author_id,
            body,
            // `github` keeps egress from posting the notice back onto the
            // pull request it is about.
            metadata: serde_json::json!({
                "github": {
                    "repo": self.repo,
                    "issue": self.pull_number,
                    "closed_unmerged": !self.merged,
                }
            }),
            content: None,
        };
        match state.store.post_message_with_event(new, None).await {
            Ok((_, stored)) => crate::routes::publish_stored(state, stored).await,
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
