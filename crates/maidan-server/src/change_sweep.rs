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
//! The swarm-metrics read shares the refresh but only records closes: it runs
//! inside a reader's request, and landing a thread there would attribute the
//! close to whoever looked. The sweep runs in no request, so the landing is
//! the system's.

use std::time::Duration;

use maidan_types::{ChangeSettlement, MemberId, ThreadId, WorkspaceId};

use crate::state::AppState;

/// Default seconds between sweeps.
pub const DEFAULT_SWEEP_SECS: u64 = 120;

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
}

/// Read `MAIDAN_CHANGE_SWEEP_SECS`. An unset or unparsable value keeps the
/// 120 s default and `0` turns the sweep off (`None`).
pub fn config_from_env() -> Option<Duration> {
    interval_from_raw(std::env::var("MAIDAN_CHANGE_SWEEP_SECS").ok())
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
            .mark_change_pull_checked(&repo, pull_number, chrono::Utc::now())
            .await
        {
            tracing::warn!(repo, pull_number, error = %err, "change sweep: check time not recorded");
        }
        let brief = match git.pull_brief(&repo, pull_number).await {
            Ok(brief) => brief,
            Err(err) => {
                tracing::debug!(repo, pull_number, error = %err, "change sweep: pull request not refreshed");
                continue;
            }
        };
        if brief.open {
            continue;
        }
        let closed_at = brief
            .closed_at
            .or(brief.merged_at)
            .unwrap_or_else(chrono::Utc::now);
        let merged_head = brief.merged_at.map(|_| brief.head_sha.as_str());
        if let Err(err) = state
            .store
            .close_change_pull(&repo, pull_number, brief.merged_at, merged_head, closed_at)
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
        refresh_open_pulls(state, workspace_id).await;
        settle_closed_pulls(state, workspace_id, &mut stats).await;
    }
    stats
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
        match state.store.settle_change_pull(&repo, pull_number).await {
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
    loop {
        sweep_once(&state).await;
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
}
