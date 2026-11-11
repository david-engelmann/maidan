//! Background projector-egress worker.
//!
//! Drains the `maidan_egress_outbox` queue: each tick claims due `pending`
//! deliveries and posts them through the configured projector sender —
//! [`SlackSender`](crate::slack::SlackSender) or
//! [`GithubSender`](crate::github::GithubSender) — marking each delivered, or,
//! on failure, rescheduled with exponential backoff, or dead-lettered once it
//! has exhausted [`MAX_ATTEMPTS`].
//!
//! Replaces the best-effort inline post the Slack (309) and GitHub (312)
//! projectors did, where a transient 502 dropped the message with a log line:
//! `route_message_to_slack` / `route_message_to_github` now only *enqueue*.
//!
//! **Retry-then-disable:** an auth/config-class failure — GitHub
//! 401/403/404, Slack `invalid_auth`/`channel_not_found` — is not retried at
//! all. No number of attempts fixes a revoked token or a deleted channel, so
//! the link is disabled (later messages stop enqueueing), the delivery
//! dead-letters, and a `ProjectorMisconfigured` event names the surface, the
//! selector and the error.
//!
//! **Retry budget:** a retry the destination API's
//! [retry budget](crate::retry_budget) refuses is handed back unsent
//! ([`defer_egress`](maidan_store::Store)): due again a few seconds later with
//! the claim's attempt given back, so Slack or GitHub coming back from an
//! outage takes the backlog at the budget's pace, and no delivery is
//! dead-lettered for waiting.
//!
//! **Runs whenever a projector sender is configured** (spawned in `main.rs` only
//! then — and the projectors only enqueue then, so an unconfigured deployment
//! neither queues nor drains). Tick defaults to 5s, tunable via
//! `MAIDAN_EGRESS_WORKER_TICK_SECS`.
//!
//! **At-least-once:** [`claim_next_due_egress`](maidan_store::Store) leases a row
//! forward, so a worker that crashes mid-post releases it after the lease and
//! another claim retries. A duplicate comment is the lesser harm against a
//! silently dropped one — the digest polarity. Multiple replicas can run the
//! worker safely (`FOR UPDATE SKIP LOCKED` on Postgres hands each a distinct
//! row), and the queue's dedup index means they enqueue one row between them in
//! the first place.
//!
//! **Result delivery:** an outbox row with [`EgressKind::Result`]
//! updates in place when the matching `maidan_result_deliveries` row has an
//! `external_ref`, recovers a GitHub comment via the hidden `<!--
//! maidan:result:<thread_id> -->` marker if that handle is lost, and otherwise
//! posts. Projector rows (`EgressKind::Projector`) always post — they must not
//! PATCH a result comment that happens to share the issue. A result 401/403/404
//! dead-letters the delivery without disabling a projector issue-link.
//!
//! **Inline reviews:** after a successful GitHub
//! *summary* comment, the worker POSTs `POST /repos/{repo}/pulls/{n}/reviews`
//! with `commit_id = envelope head_sha` (never the live PR head), `event:
//! COMMENT`, and one inline comment per usable finding (RIGHT, post-image
//! `line_range`). A missing sha, empty findings, a non-`reviewed` status,
//! Slack, a vanished envelope, or a GitHub 404/422 skips the review
//! (`maidan_github_review_total{skipped}`). A 5xx / rate-limited 403 / 401
//! records `{failed}` so operator replay retries the review. **Neither class
//! fails the outbox** — the summary has already posted, and retrying it would
//! duplicate the issue comment on a first delivery. Review errors never
//! `disable_link` a projector issue-link. Replay PATCHes the summary and POSTs
//! another COMMENT review. Projector rows never call `create_review`.
//!
//! **Check runs:** after that same successful GitHub summary, the worker
//! POSTs `POST /repos/{repo}/check-runs` with `head_sha` from the envelope
//! (never the live PR head) and `status: completed`. A critical finding, or
//! a result that was not `reviewed`, concludes `failure`. A clean reviewed
//! result concludes `success`, which does not approve the pull request.
//! No sha, a vanished envelope, Slack, a projector row, or a GitHub 404/422
//! skips the check. A 403 or 5xx records `failed` and leaves the summary
//! delivered. Neither class fails the outbox or disables a projector link.

use std::time::Duration;

use maidan_types::{EgressKind, EgressOutbox, EgressTarget, ExternalRef, ResultDelivery};

use crate::egress_body::comment_carries_result_marker;
use crate::github::GithubIssueComment;
use crate::retry_budget::{deferred_until, Attempt};
use crate::state::AppState;

/// How far forward a claim leases a row. A projector post should finish well
/// within this; a crashed worker's row becomes re-claimable after it.
const LEASE_SECS: i64 = 120;

/// Attempts before a delivery is dead-lettered (the claim counts the current try,
/// so this bounds total posts per delivery).
const MAX_ATTEMPTS: i64 = 8;

/// Belt-and-suspenders bound on posts per tick, so a large backlog can't fire
/// unbounded API calls in one pass; the remainder drains on later ticks.
const MAX_PER_TICK: u32 = 1000;

const BACKOFF_BASE_SECS: u64 = 30;
const BACKOFF_CAP_SECS: u64 = 3600;

#[derive(Debug, Clone)]
pub struct EgressWorkerConfig {
    pub tick: Duration,
}

/// The default tick (5s), overridable via `MAIDAN_EGRESS_WORKER_TICK_SECS` (>0).
/// Like the mail worker, the egress worker is not opt-in by env — it is spawned
/// whenever a projector sender is configured — so this always returns a config.
pub fn config_from_env() -> EgressWorkerConfig {
    let secs = std::env::var("MAIDAN_EGRESS_WORKER_TICK_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(5);
    EgressWorkerConfig {
        tick: Duration::from_secs(secs),
    }
}

/// Exponential backoff for the n-th attempt (n counts the current claim, so the
/// first failure is `attempts == 1`): `base * 2^(n-1)`, capped.
fn backoff_for(attempts: i64) -> Duration {
    let exp = attempts.saturating_sub(1).clamp(0, 20) as u32;
    let secs = BACKOFF_BASE_SECS
        .saturating_mul(2u64.saturating_pow(exp))
        .min(BACKOFF_CAP_SECS);
    Duration::from_secs(secs)
}

/// The wait before the next try: the backoff, or the surface's
/// `retry-after` when that is longer. A retry inside GitHub's rate-limit
/// window only fails again and spends an attempt, so the backoff never
/// undercuts it; the cap still bounds it, so a huge `retry-after` cannot park
/// a delivery for longer than the backoff ever would.
fn retry_delay(attempts: i64, retry_after: Option<u64>) -> Duration {
    let asked = Duration::from_secs(retry_after.unwrap_or(0).min(BACKOFF_CAP_SECS));
    backoff_for(attempts).max(asked)
}

/// Outcome tallies for a sweep (for tests / logging). `disabled` counts the
/// deliveries that dead-lettered because their link was turned off — a subset of
/// the dead-lettered ones, called out because it is the actionable kind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EgressSweepStats {
    pub sent: u32,
    pub retried: u32,
    pub dead: u32,
    pub disabled: u32,
    /// Retries the retry budget held back, change replies still waiting on
    /// their branch delivery, and a suspended workspace's deliveries:
    /// rescheduled, not attempted.
    pub deferred: u32,
    /// Changes the flow refused (recorded as skipped, never retried).
    pub refused: u32,
}

/// What a delivery attempt that did not fail came to.
enum Sent {
    /// It reached the surface; the handle (if any) is how to edit it later.
    Landed(Option<ExternalRef>),
    /// A change the flow refused (the branch moved, the diff does not apply):
    /// final, recorded as a skip with the reason, and never retried.
    Refused(String),
    /// A change's Slack reply whose branch delivery has not finished: handed
    /// back unsent, without costing an attempt.
    Waiting,
}

/// How long a change's Slack reply waits before looking at its branch
/// delivery again.
const WAITING_RECHECK_SECS: i64 = 5;

/// A failed delivery attempt: what to record, and whether retrying could ever
/// help. A `misconfiguration` is a wrong token, a revoked scope, a channel that
/// no longer exists — no number of retries fixes any of those.
struct DeliveryFailure {
    message: String,
    misconfiguration: bool,
    /// The seconds the surface asked for before the next try (GitHub's
    /// `retry-after`): the reschedule waits at least this long.
    retry_after: Option<u64>,
}

impl DeliveryFailure {
    /// A failed GitHub call. A rate-limited 403 is not a misconfiguration
    /// ([`crate::github::GithubError::is_misconfiguration`]), so it is
    /// rescheduled, and no sooner than GitHub's `retry-after`.
    fn github(err: crate::github::GithubError) -> Self {
        Self {
            message: err.to_string(),
            misconfiguration: err.is_misconfiguration(),
            retry_after: err.retry_after(),
        }
    }
}

/// Post one claimed delivery through the sender for its surface.
///
/// Projector rows always post. Result rows update in place when a stored
/// [`ExternalRef`] is usable, recover a GitHub comment via the hidden body
/// marker if the ref is lost, and otherwise post. The `Ok` payload is the
/// handle to persist on the result-delivery row.
async fn deliver(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
) -> Result<Sent, DeliveryFailure> {
    if entry.kind == EgressKind::Result {
        return deliver_result(state, entry, target).await;
    }
    if entry.kind == EgressKind::Withdraw {
        return deliver_withdraw(state, entry, target).await;
    }
    // A projector message or a notice is sent exactly as it was queued, so
    // the queued body is what is screened.
    let part = if entry.kind == EgressKind::Notice {
        "notice"
    } else {
        "message"
    };
    if let Some(reason) = crate::egress_guard::screen(
        target.surface(),
        entry.thread_id,
        [(part, entry.body.as_str())],
    )
    .await
    {
        return Ok(Sent::Refused(reason));
    }
    if entry.kind == EgressKind::Notice {
        return deliver_notice(state, entry, target).await;
    }
    deliver_projector(state, target, &entry.body)
        .await
        .map(Sent::Landed)
}

/// Linked-thread projector egress: always a fresh post. Update-in-place is a
/// result-delivery behaviour; applying it here would PATCH a result comment
/// that happens to share the issue.
async fn deliver_projector(
    state: &AppState,
    target: &EgressTarget,
    body: &str,
) -> Result<Option<ExternalRef>, DeliveryFailure> {
    match target {
        EgressTarget::Slack { channel_id, .. } => {
            let Some(sender) = state.slack_sender.as_ref() else {
                return Err(DeliveryFailure {
                    message: "no slack sender configured".into(),
                    misconfiguration: false,
                    retry_after: None,
                });
            };
            // Top-level: the projector egress relays a Maidan message into the
            // linked channel, and threading those under a parent would change
            // the behaviour.
            match sender.post_message(channel_id, body, None).await {
                Ok(reference) => {
                    crate::metrics::record_slack_egress("sent");
                    Ok(reference)
                }
                Err(err) => {
                    crate::metrics::record_slack_egress("failed");
                    Err(DeliveryFailure {
                        message: err.to_string(),
                        misconfiguration: err.is_misconfiguration(),
                        retry_after: None,
                    })
                }
            }
        }
        EgressTarget::Github { repo, issue_number } => {
            let Some(sender) = state.github_sender.as_ref() else {
                return Err(DeliveryFailure {
                    message: "no github sender configured".into(),
                    misconfiguration: false,
                    retry_after: None,
                });
            };
            match sender.post_comment(repo, *issue_number, body).await {
                Ok(reference) => {
                    crate::metrics::record_github_egress("sent");
                    Ok(reference)
                }
                Err(err) => {
                    crate::metrics::record_github_egress("failed");
                    Err(DeliveryFailure::github(err))
                }
            }
        }
        // Only a result can aim at a branch or a Linear issue; a projector row
        // naming one is corrupt, and no retry fixes that.
        EgressTarget::GithubBranch { .. } | EgressTarget::Linear { .. } => Err(DeliveryFailure {
            message: format!("{} is not a projector surface", target.surface()),
            misconfiguration: true,
            retry_after: None,
        }),
    }
}

async fn deliver_result(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
) -> Result<Sent, DeliveryFailure> {
    if let EgressTarget::GithubBranch { repo, branch } = target {
        return github_branch_result(state, entry, repo, branch).await;
    }
    let row = match state
        .store
        .get_result_delivery(entry.thread_id, target)
        .await
    {
        Ok(row) => row,
        Err(err) => {
            return Err(DeliveryFailure {
                message: err.to_string(),
                misconfiguration: false,
                retry_after: None,
            });
        }
    };
    // Prefer a live rebuild so a replay after a newer result ships current
    // bytes; fall back to the outbox snapshot if the envelope is gone.
    let body = crate::result_delivery::current_delivery_body(state, entry.thread_id, target)
        .await
        .unwrap_or_else(|| entry.body.clone());
    match target {
        EgressTarget::Github { repo, issue_number } => {
            // The inline review and the check run post after the summary and
            // never undo it, so their text is screened with the summary's:
            // a credential in any of them refuses the delivery before the
            // first post.
            let posts_check_runs = state
                .github_sender
                .as_ref()
                .is_some_and(|sender| sender.posts_check_runs());
            let extras = github_extra_parts(state, entry.thread_id, posts_check_runs).await;
            let parts = std::iter::once(("comment", body.as_str()))
                .chain(extras.iter().map(|(part, text)| (*part, text.as_str())));
            if let Some(reason) =
                crate::egress_guard::screen(target.surface(), entry.thread_id, parts).await
            {
                return Ok(Sent::Refused(reason));
            }
            github_result(state, entry, row.as_ref(), repo, *issue_number, &body)
                .await
                .map(Sent::Landed)
        }
        EgressTarget::Slack {
            channel_id,
            thread_ts,
        } => {
            let body = match crate::result_delivery::change_reply(
                state,
                entry.workspace_id,
                entry.thread_id,
            )
            .await
            {
                crate::result_delivery::ChangeReply::Waiting => return Ok(Sent::Waiting),
                crate::result_delivery::ChangeReply::Ready(reply) => {
                    crate::result_delivery::slack_reply_body(&reply)
                }
                crate::result_delivery::ChangeReply::NotAChange => body,
            };
            if let Some(reason) = crate::egress_guard::screen(
                target.surface(),
                entry.thread_id,
                [("message", body.as_str())],
            )
            .await
            {
                return Ok(Sent::Refused(reason));
            }
            slack_result(state, row.as_ref(), channel_id, thread_ts.as_deref(), &body)
                .await
                .map(Sent::Landed)
        }
        EgressTarget::Linear { team_id, issue } => {
            let body = match crate::result_delivery::change_reply(
                state,
                entry.workspace_id,
                entry.thread_id,
            )
            .await
            {
                crate::result_delivery::ChangeReply::Waiting => return Ok(Sent::Waiting),
                crate::result_delivery::ChangeReply::Ready(reply) => {
                    crate::result_delivery::linear_reply_body(&reply)
                }
                crate::result_delivery::ChangeReply::NotAChange => body,
            };
            if let Some(reason) = crate::egress_guard::screen(
                target.surface(),
                entry.thread_id,
                [("comment", body.as_str())],
            )
            .await
            {
                return Ok(Sent::Refused(reason));
            }
            linear_result(state, entry, row.as_ref(), team_id, issue, &body).await
        }
        EgressTarget::GithubBranch { .. } => Err(DeliveryFailure {
            message: "github_branch is delivered by the change flow".into(),
            misconfiguration: true,
            retry_after: None,
        }),
    }
}

/// Post a notice as written: a task's claim, release or handoff packet on
/// its Linear issue, or a handoff line in its Slack thread. The allowlist is
/// read again here, so a target unblessed after the notice was queued stops
/// it, and a Linear issue must still be in the team it was queued for.
async fn deliver_notice(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
) -> Result<Sent, DeliveryFailure> {
    let allowed = |surface| async move {
        state
            .store
            .is_egress_target_allowed(entry.workspace_id, surface, &target.allowlist_selector())
            .await
            .map_err(|err| DeliveryFailure {
                message: err.to_string(),
                misconfiguration: false,
                retry_after: None,
            })
    };
    if let EgressTarget::Slack {
        channel_id,
        thread_ts,
    } = target
    {
        if !allowed(maidan_types::EgressSurface::Slack).await? {
            return Ok(Sent::Refused(format!(
                "Slack channel {channel_id} is not in the workspace egress allowlist"
            )));
        }
        return slack_result(state, None, channel_id, thread_ts.as_deref(), &entry.body)
            .await
            .map(Sent::Landed);
    }
    let EgressTarget::Linear { team_id, issue } = target else {
        return Err(DeliveryFailure {
            message: format!("{} takes no notice", target.surface()),
            misconfiguration: true,
            retry_after: None,
        });
    };
    let Some(sender) = state.linear_sender.as_ref() else {
        return Err(DeliveryFailure {
            message: "no linear sender configured".into(),
            misconfiguration: false,
            retry_after: None,
        });
    };
    if !allowed(maidan_types::EgressSurface::Linear).await? {
        return Ok(Sent::Refused(format!(
            "Linear team {team_id} is not in the workspace egress allowlist"
        )));
    }
    let found = sender.issue(issue).await.map_err(linear_failure)?;
    if found.team_id != *team_id {
        return Ok(Sent::Refused(format!(
            "{} is no longer in team {team_id}",
            found.identifier
        )));
    }
    let comment_id = sender
        .create_comment(&found.id, &entry.body)
        .await
        .map_err(linear_failure)?;
    Ok(Sent::Landed(Some(ExternalRef::Linear { comment_id })))
}

/// The withdrawal comment when the queued one cannot be sent as written.
const WITHDRAWN_WITHOUT_REASON: &str = "Maidan closed this pull request because its task ended \
     without landing. It is out of the merge queue; its branch is left as it is.";

/// The hidden marker at byte 0 of a withdrawal's comment. It names the outbox
/// row, so a retry of this withdrawal finds its own comment and posts none,
/// while a later withdrawal of the same pull request still says why.
fn withdrawal_marker(entry: &EgressOutbox) -> String {
    format!("<!-- maidan:withdraw:{} -->", entry.id.0)
}

/// How far before the row's own clock reading GitHub's comment times may
/// fall: the two clocks differ, and a comment read twice costs nothing.
const WITHDRAWAL_CLOCK_MARGIN: chrono::Duration = chrono::Duration::minutes(10);

/// When the withdrawal was queued, less [`WITHDRAWAL_CLOCK_MARGIN`]. Its
/// comment cannot predate the row, and the row's id is a v7 UUID, which
/// carries its creation time, so no column is needed to scan from there.
fn withdrawal_queued_at(entry: &EgressOutbox) -> Option<chrono::DateTime<chrono::Utc>> {
    let (secs, nanos) = entry.id.0.get_timestamp()?.to_unix();
    let queued = chrono::DateTime::from_timestamp(i64::try_from(secs).ok()?, nanos)?;
    queued.checked_sub_signed(WITHDRAWAL_CLOCK_MARGIN)
}

/// Withdraw a cancelled or abandoned task's pull request: take it out of the
/// merge queue and close it with the queued comment, through the withdraw
/// guard path. The cancel or close queued this in its own transaction for a
/// pull request the change flow opened for the task. One that another task
/// answers for now (its result was delivered there since) is left alone, as
/// is anything the guards refuse: it is no longer this task's to close.
async fn deliver_withdraw(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
) -> Result<Sent, DeliveryFailure> {
    let EgressTarget::Github { repo, issue_number } = target else {
        return Err(DeliveryFailure {
            message: format!("{} takes no withdrawal", target.surface()),
            misconfiguration: true,
            retry_after: None,
        });
    };
    let Some(sender) = state.github_sender.as_deref() else {
        return Err(DeliveryFailure {
            message: "github is not configured to write".into(),
            misconfiguration: true,
            retry_after: None,
        });
    };
    let Some(git) = sender.git() else {
        return Err(DeliveryFailure {
            message: "github is not configured to write".into(),
            misconfiguration: true,
            retry_after: None,
        });
    };
    let store_failure = |err: maidan_store::StoreError| DeliveryFailure {
        message: err.to_string(),
        misconfiguration: false,
        retry_after: None,
    };
    let answering = state
        .store
        .change_pull_thread(entry.workspace_id, repo, *issue_number)
        .await
        .map_err(store_failure)?;
    if answering != Some(entry.thread_id) {
        return Ok(Sent::Refused(format!(
            "{repo}#{issue_number} no longer answers to this task, so it is left as it is"
        )));
    }
    // The comment carries the canceller's reason. A reason the credential
    // guard refuses must not keep the pull request in the merge queue, so it
    // is withdrawn anyway, with a comment that leaves the reason out.
    let comment = match crate::egress_guard::screen(
        target.surface(),
        entry.thread_id,
        [("notice", entry.body.as_str())],
    )
    .await
    {
        None => entry.body.as_str(),
        Some(_) => WITHDRAWN_WITHOUT_REASON,
    };
    let marker = withdrawal_marker(entry);
    match crate::routes::withdraw_pull_guarded(
        state.store.as_ref(),
        entry.workspace_id,
        git,
        &state.mark_ready_bases,
        &state.merge_queue_labels,
        repo,
        *issue_number,
        crate::routes::WithdrawalComment {
            reader: sender,
            marker: &marker,
            text: comment,
            since: withdrawal_queued_at(entry),
        },
    )
    .await
    {
        Ok(withdrawn) => {
            crate::audit::record(
                state,
                maidan_types::NewAuditEvent {
                    scope: maidan_types::AuditScope::Workspace(entry.workspace_id),
                    actor_id: None,
                    action: "github.withdraw".into(),
                    target_kind: Some("github_pull".into()),
                    target_id: None,
                    metadata: serde_json::json!({
                        "repo": repo,
                        "pull_number": withdrawn.number,
                        "thread_id": entry.thread_id.0,
                        "dequeued": withdrawn.dequeued,
                        "closed": withdrawn.closed,
                    }),
                },
            )
            .await;
            Ok(Sent::Landed(None))
        }
        Err(crate::routes::MarkReadyGuard::Refused(reason)) => Ok(Sent::Refused(reason)),
        Err(crate::routes::MarkReadyGuard::Github(err)) => Err(DeliveryFailure::github(err)),
        Err(
            crate::routes::MarkReadyGuard::Store(err)
            | crate::routes::MarkReadyGuard::ThreadEnded(err),
        ) => Err(store_failure(err)),
    }
}

fn linear_failure(err: crate::linear::LinearError) -> DeliveryFailure {
    DeliveryFailure {
        message: err.to_string(),
        misconfiguration: err.is_misconfiguration(),
        retry_after: None,
    }
}

/// Comment on the Linear issue, then move it to the state the result asked
/// for.
///
/// The allowlist blessed the team the result names, not the issue, so the
/// issue is looked up first and refused unless it is in that team: a result
/// naming a blessed team and another team's issue would otherwise write
/// wherever the key reaches. A comment posted earlier is edited in place; one
/// deleted since is posted again. The state move comes after the comment and
/// never undoes it: a state the team does not have, or a move Linear refuses,
/// is logged and the comment stands.
async fn linear_result(
    state: &AppState,
    entry: &EgressOutbox,
    row: Option<&ResultDelivery>,
    team_id: &str,
    issue: &str,
    body: &str,
) -> Result<Sent, DeliveryFailure> {
    let Some(sender) = state.linear_sender.as_ref() else {
        return Err(DeliveryFailure {
            message: "no linear sender configured".into(),
            misconfiguration: false,
            retry_after: None,
        });
    };
    let found = sender.issue(issue).await.map_err(linear_failure)?;
    if found.team_id != team_id {
        return Ok(Sent::Refused(format!(
            "{} is not in team {team_id}, the team the result names and the allowlist blessed",
            found.identifier
        )));
    }
    let edited = match row.and_then(|r| r.reference()) {
        Some(ExternalRef::Linear { comment_id }) => {
            match sender.update_comment(&comment_id, body).await {
                Ok(()) => Some(comment_id),
                Err(crate::linear::LinearError::NotFound(_)) => None,
                Err(err) => return Err(linear_failure(err)),
            }
        }
        _ => None,
    };
    let comment_id = match edited {
        Some(id) => id,
        None => sender
            .create_comment(&found.id, body)
            .await
            .map_err(linear_failure)?,
    };
    if let Some(wanted) = requested_linear_state(state, entry.thread_id, team_id, issue).await {
        match found.state_named(&wanted) {
            Some(target_state) => {
                if let Err(err) = sender.move_issue(&found.id, &target_state.id).await {
                    tracing::warn!(error = %err, issue = %found.identifier, state = %wanted, "linear delivery: the comment landed, the state move failed");
                }
            }
            None => tracing::warn!(
                issue = %found.identifier,
                state = %wanted,
                "linear delivery: the comment landed; the team has no state with that name"
            ),
        }
    }
    Ok(Sent::Landed(Some(ExternalRef::Linear { comment_id })))
}

/// The workflow state the result asks this issue to move to, read from the
/// envelope as it is now.
async fn requested_linear_state(
    state: &AppState,
    thread_id: maidan_types::ThreadId,
    team_id: &str,
    issue: &str,
) -> Option<String> {
    let waiter = crate::result_delivery::current_waiter(state, thread_id).await?;
    waiter
        .deliver_to
        .into_iter()
        .find_map(|target| match target {
            maidan_types::DeliverTarget::Linear {
                team,
                issue: i,
                state: Some(wanted),
            } if team == team_id && i == issue => Some(wanted),
            _ => None,
        })
}

/// Commit a change result to its branch and open its draft pull request.
///
/// The commit is built from the envelope as it is now: `base_sha`, the diff
/// and the PR's `base` all come from the result, never from the outbox
/// snapshot or the thread. A result that is gone, or no longer a change for
/// this branch, is refused rather than retried.
/// GitHub's merge queue can drop a pull request (a merge conflict, failed
/// checks) and leave the queue's label on it. The change flow refuses to push
/// to a labelled pull request, so the round that fixes the conflict would be
/// refused for good, and the label would also make a later enqueue a no-op.
/// When GitHub's own read says the pull request left the queue and is not
/// back in it, the label is stale: it comes off through the dequeue guard
/// path. That way the new head is not queued unreviewed, and the mark-ready
/// app's next enqueue labels it again. Anything else (still queued, no
/// removal event, a guard refusal) leaves the label for the change flow to
/// refuse on, as before.
async fn clear_stale_queue_label(
    state: &AppState,
    git: &dyn crate::github::GithubGit,
    entry: &EgressOutbox,
    repo: &str,
    branch: &str,
) -> Result<(), DeliveryFailure> {
    let Some(label) = state.merge_queue_labels.label_for(repo) else {
        return Ok(());
    };
    let failure = DeliveryFailure::github;
    let Some(pull) = git.open_pull(repo, branch).await.map_err(failure)? else {
        return Ok(());
    };
    if !pull.labels.iter().any(|name| name == label) {
        return Ok(());
    }
    // Only the merge-queue gate is used here, so the Actions fallback for
    // the checks would be calls spent for nothing.
    let snapshot = git
        .pull_snapshot(repo, pull.number, crate::github::ActionsRead::Skip)
        .await
        .map_err(failure)?;
    let Some(gate) = snapshot.gate else {
        return Ok(());
    };
    let Some(reason) = gate.dequeued.filter(|_| !gate.queued) else {
        return Ok(());
    };
    match crate::routes::dequeue_pull_guarded(
        state.store.as_ref(),
        entry.workspace_id,
        git,
        &state.mark_ready_bases,
        &state.merge_queue_labels,
        repo,
        pull.number,
    )
    .await
    {
        Ok((number, label, removed)) => {
            if removed {
                crate::audit::record(
                    state,
                    maidan_types::NewAuditEvent {
                        scope: maidan_types::AuditScope::Workspace(entry.workspace_id),
                        actor_id: None,
                        action: "github.enqueue".into(),
                        target_kind: Some("github_pull".into()),
                        target_id: None,
                        metadata: serde_json::json!({
                            "repo": repo,
                            "pull_number": number,
                            "outcome": "stale_label_cleared",
                            "label": label,
                            "reason": reason,
                        }),
                    },
                )
                .await;
            }
            Ok(())
        }
        Err(crate::routes::MarkReadyGuard::Github(err)) => Err(failure(err)),
        // A guard refusal or a store error leaves the label on; the change
        // flow then refuses this round as it did before.
        Err(err) => {
            tracing::warn!(
                ?err,
                repo,
                number = pull.number,
                "egress worker: stale merge-queue label left on"
            );
            Ok(())
        }
    }
}

async fn github_branch_result(
    state: &AppState,
    entry: &EgressOutbox,
    repo: &str,
    branch: &str,
) -> Result<Sent, DeliveryFailure> {
    let Some(sender) = state.github_sender.as_ref() else {
        return Err(DeliveryFailure {
            message: "no github sender configured".into(),
            misconfiguration: false,
            retry_after: None,
        });
    };
    let Some(git) = sender.git() else {
        return Err(DeliveryFailure {
            message: "the github sender cannot write branches".into(),
            misconfiguration: true,
            retry_after: None,
        });
    };
    let stored = match state.store.get_thread_result(entry.thread_id).await {
        Ok(Some(stored)) => stored,
        Ok(None) => {
            return Ok(Sent::Refused(
                "the result is no longer on the thread".into(),
            ));
        }
        Err(err) => {
            return Err(DeliveryFailure {
                message: err.to_string(),
                misconfiguration: false,
                retry_after: None,
            });
        }
    };
    let Some(change) = maidan_types::parse_change_result(&stored.result) else {
        return Ok(Sent::Refused(
            "the result on the thread is not a change result".into(),
        ));
    };
    let targets = maidan_types::parse_waiter_result(&stored.result)
        .map(|waiter| waiter.deliver_to)
        .unwrap_or_default();
    let base = targets.iter().find_map(|target| match target {
        maidan_types::DeliverTarget::GithubBranch {
            repo: r,
            branch: b,
            base,
        } if r == repo && b == branch => Some(base.clone()),
        _ => None,
    });
    // The issue the producer names, else the one the result is delivered to.
    let linear_issue = change.linear_issue.clone().or_else(|| {
        targets.iter().find_map(|target| match target {
            maidan_types::DeliverTarget::Linear { issue, .. }
                if maidan_types::is_issue_key(issue) =>
            {
                Some(issue.clone())
            }
            _ => None,
        })
    });
    let Some(base) = base else {
        return Ok(Sent::Refused(
            "the result no longer targets this branch".into(),
        ));
    };
    // Checked again at send time, against the allowlist as it is now: a
    // blessing revoked after the result was routed stops the write.
    if let Err(reason) = maidan_types::check_change_target(branch, &base) {
        return Ok(Sent::Refused(reason));
    }
    let selector = maidan_types::change_allowlist_selector(repo, &base);
    match state
        .store
        .is_egress_target_allowed(
            entry.workspace_id,
            maidan_types::EgressSurface::GithubBranch,
            &selector,
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return Ok(Sent::Refused(format!(
                "`{selector}` is not in the workspace egress allowlist"
            )));
        }
        Err(err) => {
            return Err(DeliveryFailure {
                message: err.to_string(),
                misconfiguration: false,
                retry_after: None,
            });
        }
    }
    // Only read when the result has no title; a failed read falls back to
    // the branch name rather than holding the delivery.
    let opening = if change.title.is_none() {
        state
            .store
            .list_messages_after(entry.thread_id, None, 1)
            .await
            .ok()
            .and_then(|messages| messages.into_iter().next())
            .map(|m| m.body)
    } else {
        None
    };
    // Read again here, after every other check and before the first GitHub
    // call, so a cancel that landed while the delivery waited in the queue
    // stops the commit and the draft pull request. Read once: a cancel that
    // commits after this point does not stop a delivery already writing.
    if let Some(reason) = ended_change_reason(state, entry.thread_id).await? {
        return Ok(Sent::Refused(reason));
    }
    // Only a squash-rebase needs it: the force move is allowed only while the
    // branch head is still the last commit this flow delivered to the pull
    // request, as recorded for this workspace.
    let delivered_head = if change.rebase && change.is_changed() {
        delivered_head(state, git, entry, repo, branch).await?
    } else {
        None
    };
    // A `no_change` round only posts its check, which no queue label holds
    // up, so it leaves the pull request alone.
    if !change.is_check_only() {
        clear_stale_queue_label(state, git, entry, repo, branch).await?;
    }
    let request = crate::change_flow::ChangeRequest {
        repo,
        branch,
        base: &base,
        thread_id: entry.thread_id,
        change: &change,
        opening_message: opening.as_deref(),
        merge_queue_label: state.merge_queue_labels.label_for(repo),
        delivered_head,
    };
    match crate::change_flow::deliver_change(git, &request).await {
        Ok(crate::change_flow::ChangeOutcome::Committed {
            commit_sha,
            pull,
            rebased_from,
            template_warnings: mut warnings,
            check_warning,
        }) => {
            crate::metrics::record_github_egress("sent");
            // A `check` that did not become a commit status rides with the
            // template flags: the change landed either way.
            warnings.extend(check_warning.map(str::to_string));
            // Before the row is marked delivered, so the Slack reply that
            // waits on it reads them. Best effort: a flag never holds a
            // landed change.
            let target = EgressTarget::GithubBranch {
                repo: repo.to_string(),
                branch: branch.to_string(),
            };
            if let Err(err) = state
                .store
                .set_result_delivery_warnings(entry.thread_id, &target, &warnings)
                .await
            {
                tracing::warn!(error = %err, thread_id = ?entry.thread_id, "change flow: delivery warnings not recorded");
            }
            let landed = RoundLanded {
                commit_sha: &commit_sha,
                pull_number: pull.number,
                previous_head: rebased_from,
                linear_issue,
            };
            record_change_round(state, git, entry, &request, landed).await;
            Ok(Sent::Landed(Some(ExternalRef::GithubBranch {
                repo: repo.to_string(),
                commit_sha,
                pull_number: pull.number,
            })))
        }
        Ok(crate::change_flow::ChangeOutcome::CheckOnly {
            head_sha,
            check_warning,
        }) => {
            if check_warning.is_none() {
                crate::metrics::record_github_egress("sent");
            }
            // Recorded before the row is marked delivered, as for a commit,
            // so the reply reads it; and replacing the last round's flags,
            // which described a pull request this round never touched.
            let warnings: Vec<String> = check_warning.map(str::to_string).into_iter().collect();
            let target = EgressTarget::GithubBranch {
                repo: repo.to_string(),
                branch: branch.to_string(),
            };
            if let Err(err) = state
                .store
                .set_result_delivery_warnings(entry.thread_id, &target, &warnings)
                .await
            {
                tracing::warn!(error = %err, thread_id = ?entry.thread_id, "change flow: check warning not recorded");
            }
            tracing::info!(thread_id = ?entry.thread_id, repo, branch, head_sha = %head_sha, ?check_warning, "change flow: no_change round's check delivered");
            Ok(Sent::Landed(None))
        }
        Ok(crate::change_flow::ChangeOutcome::Refused(reason)) => Ok(Sent::Refused(reason)),
        Err(err) => {
            crate::metrics::record_github_egress("failed");
            let workflows = change
                .diff
                .as_deref()
                .map(maidan_types::workflow_paths)
                .unwrap_or_default();
            Err(branch_failure(err, &workflows))
        }
    }
}

/// A failed branch write, read against the workflow files the change edits.
///
/// GitHub answers a push that touches [`maidan_types::WORKFLOW_DIR`] with a
/// bare 403 (`Resource not accessible by personal access token`) when the
/// token lacks the Workflows permission, which names neither the file nor
/// the fix. A 403 that refuses access, on a change that edits a workflow
/// file, is final: no retry gets a permission back, so it dead-letters at
/// once with a reason that names the files and both ways out. A rate-limited
/// 403 still retries, and names the files too, so whoever reads it later
/// does not have to dig the diff out to see what was being pushed.
fn branch_failure(err: crate::github::GithubError, workflows: &[String]) -> DeliveryFailure {
    let forbidden = matches!(err, crate::github::GithubError::Api { status: 403, .. });
    let refused = forbidden && err.is_permission_refusal();
    let mut failure = DeliveryFailure::github(err);
    if !forbidden || workflows.is_empty() {
        return failure;
    }
    let named = maidan_types::named_paths(workflows);
    if refused {
        failure.misconfiguration = true;
        failure.retry_after = None;
        failure.message = format!(
            "{}; the change edits {named}, and GitHub refuses a workflow file to a delivery token without the Workflows permission. Drop the workflow edit from the diff and set the result again, or give the delivery token the Workflows permission (the `workflow` scope on a classic token)",
            failure.message
        );
    } else {
        failure.message = format!("{}; the change edits {named}", failure.message);
    }
    failure
}

/// Why a change delivery must not write to GitHub, or `None` when it may.
/// The reason starts with `thread_ended`, so the delivery row, the producer
/// and the Slack reply all show it.
///
/// A thread that is archived or gone takes no more work. A closed one is
/// told apart by how it closed, not by its state: a producer that lands its
/// change and closes the task at once reaches this point with the thread
/// already closed, and that delivery must still go out. A cancel and a
/// close-abandoned each leave a record of an end without landing, and a
/// closed thread can never be reopened, so a closed thread with neither
/// record delivers. Only this branch write reads the thread; a message relay
/// still goes out, so a cancel's own notice reaches Slack.
async fn ended_change_reason(
    state: &AppState,
    thread_id: maidan_types::ThreadId,
) -> Result<Option<String>, DeliveryFailure> {
    let store_failure = |err: maidan_store::StoreError| DeliveryFailure {
        message: err.to_string(),
        misconfiguration: false,
        retry_after: None,
    };
    let thread = match state.store.get_thread(thread_id).await {
        Ok(thread) => thread,
        Err(maidan_store::StoreError::NotFound) => {
            return Ok(Some(
                "thread_ended: the task no longer exists, so nothing was pushed".into(),
            ));
        }
        Err(err) => return Err(store_failure(err)),
    };
    let ended = if thread.tombstoned_at.is_some() {
        Some("was deleted".to_string())
    } else {
        match thread.state {
            maidan_types::ThreadState::Archived => Some("is archived".to_string()),
            maidan_types::ThreadState::Closed => state
                .store
                .thread_unlanded_close(thread_id)
                .await
                .map_err(store_failure)?
                .map(|how| match how.as_str() {
                    "cancelled" => "was cancelled".to_string(),
                    reason => format!("was closed as {reason}"),
                }),
            _ => None,
        }
    };
    Ok(ended.map(|how| {
        format!(
            "thread_ended: the task {how} after this change was queued, so nothing was pushed and no pull request was opened"
        )
    }))
}

/// The newest round recorded for the branch's open pull request in this
/// workspace, or `None` when no pull request is open or nothing was recorded.
async fn delivered_head(
    state: &AppState,
    git: &dyn crate::github::GithubGit,
    entry: &EgressOutbox,
    repo: &str,
    branch: &str,
) -> Result<Option<crate::change_flow::DeliveredHead>, DeliveryFailure> {
    let pull = git
        .open_pull(repo, branch)
        .await
        .map_err(DeliveryFailure::github)?;
    let Some(pull) = pull else {
        return Ok(None);
    };
    let head = state
        .store
        .latest_change_round_head(entry.workspace_id, repo, pull.number)
        .await
        .map_err(|err| DeliveryFailure {
            message: err.to_string(),
            misconfiguration: false,
            retry_after: None,
        })?;
    Ok(head.map(|commit_sha| crate::change_flow::DeliveredHead {
        pull_number: pull.number,
        commit_sha,
    }))
}

/// The text the inline review and the check run would publish for this
/// result as it is now: what [`post_result_inline_review`] and
/// [`post_result_check_run`] build. A result that cannot be read gives none;
/// those posts then skip for the same reason. The check run counts only when
/// the sender can post one: with a token it is skipped, and its text must not
/// hold up a clean comment.
async fn github_extra_parts(
    state: &AppState,
    thread_id: maidan_types::ThreadId,
    posts_check_runs: bool,
) -> Vec<(&'static str, String)> {
    let Ok(Some(stored)) = state.store.get_thread_result(thread_id).await else {
        return Vec::new();
    };
    let Some(waiter) = maidan_types::parse_waiter_result(&stored.result) else {
        return Vec::new();
    };
    let mut parts: Vec<(&'static str, String)> =
        crate::result_delivery::prepare_inline_review(&waiter)
            .map(|review| {
                review
                    .comments
                    .into_iter()
                    .map(|comment| ("inline review", review_comment_text(&comment)))
                    .collect()
            })
            .unwrap_or_default();
    if let Some(check) =
        maidan_types::github_check_run(&waiter, &stored.result).filter(|_| posts_check_runs)
    {
        parts.push(("check run", check_run_text(&check)));
    }
    parts
}

/// What an inline review comment publishes: its path as well as its body.
fn review_comment_text(comment: &maidan_types::GithubReviewComment) -> String {
    format!("{}\n{}", comment.path, comment.body)
}

/// Every producer-written field of a check run, as one text to screen.
fn check_run_text(check: &maidan_types::GithubCheckRun) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        check.name,
        check.title,
        check.summary,
        check.details_url.as_deref().unwrap_or_default()
    )
}

async fn github_result(
    state: &AppState,
    entry: &EgressOutbox,
    row: Option<&ResultDelivery>,
    repo: &str,
    issue_number: i64,
    body: &str,
) -> Result<Option<ExternalRef>, DeliveryFailure> {
    let Some(sender) = state.github_sender.as_ref() else {
        return Err(DeliveryFailure {
            message: "no github sender configured".into(),
            misconfiguration: false,
            retry_after: None,
        });
    };
    let reference =
        deliver_github_result_comment(sender.as_ref(), entry, row, repo, issue_number, body)
            .await?;
    // Additive: a review or check-run skip/failure never undoes a landed summary.
    post_result_inline_review(state, sender.as_ref(), repo, issue_number, entry.thread_id).await;
    post_result_check_run(state, sender.as_ref(), repo, entry.thread_id).await;
    // Only a review thread's result: it may post its guarded `pre-review`
    // status, and records why not on this row.
    crate::pre_review::post_status(state, sender.as_ref(), entry, repo, issue_number).await;
    Ok(reference)
}

/// The result summary comment: update in place, recover via the marker, or
/// post. Returns the handle to persist; does not post inline findings.
async fn deliver_github_result_comment(
    sender: &dyn crate::github::GithubSender,
    entry: &EgressOutbox,
    row: Option<&ResultDelivery>,
    repo: &str,
    issue_number: i64,
    body: &str,
) -> Result<Option<ExternalRef>, DeliveryFailure> {
    if let Some(ExternalRef::Github { comment_id, .. }) = row.and_then(|r| r.reference()) {
        match sender.update_comment(repo, comment_id, body).await {
            Ok(()) => {
                crate::metrics::record_github_egress("sent");
                return Ok(Some(ExternalRef::Github {
                    repo: repo.to_string(),
                    comment_id,
                }));
            }
            Err(err) if err.is_not_found() => {
                // Comment deleted — fall through to marker recovery, then post.
            }
            Err(err) => {
                crate::metrics::record_github_egress("failed");
                return Err(DeliveryFailure::github(err));
            }
        }
    }
    // Recover via the hidden marker only when we have previously landed
    // something (a lost ref, or a post that GitHub accepted without an id).
    // A first delivery just posts — listing every issue comment on every
    // first review would be an unbounded walk for no gain.
    if row.is_some_and(|r| r.delivered_revision.is_some()) {
        match recover_github_comment(sender, repo, issue_number, entry.thread_id).await {
            Ok(Some(comment_id)) => match sender.update_comment(repo, comment_id, body).await {
                Ok(()) => {
                    crate::metrics::record_github_egress("sent");
                    return Ok(Some(ExternalRef::Github {
                        repo: repo.to_string(),
                        comment_id,
                    }));
                }
                Err(err) if err.is_not_found() => {}
                Err(err) => {
                    crate::metrics::record_github_egress("failed");
                    return Err(DeliveryFailure::github(err));
                }
            },
            Ok(None) => {}
            Err(failure) => return Err(failure),
        }
    }
    match sender.post_comment(repo, issue_number, body).await {
        Ok(reference) => {
            crate::metrics::record_github_egress("sent");
            Ok(reference)
        }
        Err(err) => {
            crate::metrics::record_github_egress("failed");
            Err(DeliveryFailure::github(err))
        }
    }
}

/// Post a COMMENT review for the current waiter envelope.
///
/// Never returns an error. The summary comment has already landed; failing the
/// outbox here would retry into a duplicate issue comment on a first delivery.
/// 404 (issue is not a PR) and 422 (line not in the diff at `head_sha`) are
/// metric-skips. A 5xx / rate-limited 403 / 401 is logged as `failed` and left
/// for operator replay, which PATCHes the summary and POSTs another COMMENT
/// review. Neither path calls `disable_link`.
async fn post_result_inline_review(
    state: &AppState,
    sender: &dyn crate::github::GithubSender,
    repo: &str,
    issue_number: i64,
    thread_id: maidan_types::ThreadId,
) {
    let Some(waiter) = crate::result_delivery::current_waiter(state, thread_id).await else {
        crate::metrics::record_github_review("skipped");
        return;
    };
    let Some(review) = crate::result_delivery::prepare_inline_review(&waiter) else {
        crate::metrics::record_github_review("skipped");
        return;
    };
    // Screened with the summary already; read again here because a newer
    // result can have replaced the envelope since.
    let texts: Vec<String> = review.comments.iter().map(review_comment_text).collect();
    if let Some(kind) = crate::egress_guard::first_credential(texts.iter().map(String::as_str)) {
        crate::metrics::record_github_review("skipped");
        crate::egress_guard::report_late_skip(
            maidan_types::EgressSurface::Github,
            thread_id,
            "inline review",
            kind,
        )
        .await;
        return;
    }
    if review.truncated {
        tracing::warn!(
            %repo,
            pull = issue_number,
            kept = review.comments.len(),
            "github inline review: capped findings at GitHub's 100-comment limit"
        );
    }
    match sender
        .create_review(repo, issue_number, &review.commit_id, &review.comments)
        .await
    {
        Ok(()) => {
            crate::metrics::record_github_review("sent");
            tracing::debug!(
                %repo,
                pull = issue_number,
                commit_id = %review.commit_id,
                comments = review.comments.len(),
                "github inline review: posted"
            );
        }
        Err(err) => {
            // The summary stays. 404/422 will not recover on replay; 5xx/auth
            // will. Neither fails the outbox or disables a projector link.
            let outcome = if err.is_inline_review_skip() {
                "skipped"
            } else {
                "failed"
            };
            crate::metrics::record_github_review(outcome);
            tracing::warn!(
                error = %err,
                %repo,
                pull = issue_number,
                commit_id = %review.commit_id,
                outcome,
                "github inline review: not posted; summary comment still delivered"
            );
        }
    }
}

/// Post a completed check run for the current waiter envelope.
///
/// Never returns an error. Same polarity as [`post_result_inline_review`]:
/// the summary comment has already landed. `head_sha` comes only from the
/// envelope. A 404/422 will not recover on replay and is `skipped`; anything
/// else is `failed` and left for operator replay, which posts another check
/// on the same sha. Neither path calls `disable_link`.
async fn post_result_check_run(
    state: &AppState,
    sender: &dyn crate::github::GithubSender,
    repo: &str,
    thread_id: maidan_types::ThreadId,
) {
    let stored = match state.store.get_thread_result(thread_id).await {
        Ok(stored) => stored,
        Err(err) => {
            crate::github::record_github_check_run("skipped");
            tracing::warn!(error = %err, "github check run: result lookup failed");
            return;
        }
    };
    let Some(stored) = stored else {
        crate::github::record_github_check_run("skipped");
        return;
    };
    let Some(waiter) = maidan_types::parse_waiter_result(&stored.result) else {
        crate::github::record_github_check_run("skipped");
        return;
    };
    let Some(check) = maidan_types::github_check_run(&waiter, &stored.result) else {
        crate::github::record_github_check_run("skipped");
        return;
    };
    // Only a GitHub App may create check runs; with a token the call is a
    // guaranteed 403. Boot says so once; the summary comment carries the
    // verdict.
    if !sender.posts_check_runs() {
        crate::github::record_github_check_run("skipped");
        tracing::debug!(%repo, "github check run: skipped; a token cannot post check runs");
        return;
    }
    // As for the inline review: screened with the summary, read again here.
    if let Some(kind) = crate::egress_guard::first_credential([check_run_text(&check).as_str()]) {
        crate::github::record_github_check_run("skipped");
        crate::egress_guard::report_late_skip(
            maidan_types::EgressSurface::Github,
            thread_id,
            "check run",
            kind,
        )
        .await;
        return;
    }
    match sender.create_check_run(repo, &check).await {
        Ok(()) => {
            crate::github::record_github_check_run("sent");
            tracing::debug!(
                %repo,
                head_sha = %check.head_sha,
                conclusion = check.conclusion.as_str(),
                "github check run: posted"
            );
        }
        Err(err) => {
            let outcome = if err.is_not_found() || err.is_unprocessable() {
                "skipped"
            } else {
                "failed"
            };
            crate::github::record_github_check_run(outcome);
            tracing::warn!(
                error = %err,
                %repo,
                head_sha = %check.head_sha,
                outcome,
                "github check run: not posted; summary comment still delivered"
            );
        }
    }
}

async fn recover_github_comment(
    sender: &dyn crate::github::GithubSender,
    repo: &str,
    issue_number: i64,
    thread_id: maidan_types::ThreadId,
) -> Result<Option<i64>, DeliveryFailure> {
    match sender.list_issue_comments(repo, issue_number).await {
        Ok(comments) => Ok(comments.into_iter().find_map(|c: GithubIssueComment| {
            comment_carries_result_marker(&c.body, thread_id).then_some(c.id)
        })),
        Err(err) => {
            crate::metrics::record_github_egress("failed");
            Err(DeliveryFailure::github(err))
        }
    }
}

async fn slack_result(
    state: &AppState,
    row: Option<&ResultDelivery>,
    channel_id: &str,
    thread_ts: Option<&str>,
    body: &str,
) -> Result<Option<ExternalRef>, DeliveryFailure> {
    let Some(sender) = state.slack_sender.as_ref() else {
        return Err(DeliveryFailure {
            message: "no slack sender configured".into(),
            misconfiguration: false,
            retry_after: None,
        });
    };
    if let Some(ExternalRef::Slack { ts, .. }) = row.and_then(|r| r.reference()) {
        match sender.update_message(channel_id, &ts, body).await {
            Ok(()) => {
                crate::metrics::record_slack_egress("sent");
                return Ok(Some(ExternalRef::Slack {
                    channel_id: channel_id.to_string(),
                    ts,
                }));
            }
            Err(err) if err.is_message_gone() => {
                // No Slack marker. A lost message is a new post, which may
                // duplicate — the documented trade against silence.
            }
            Err(err) => {
                crate::metrics::record_slack_egress("failed");
                return Err(DeliveryFailure {
                    message: err.to_string(),
                    misconfiguration: err.is_misconfiguration(),
                    retry_after: None,
                });
            }
        }
    }
    match sender.post_message(channel_id, body, thread_ts).await {
        Ok(reference) => {
            crate::metrics::record_slack_egress("sent");
            Ok(reference)
        }
        Err(err) => {
            crate::metrics::record_slack_egress("failed");
            Err(DeliveryFailure {
                message: err.to_string(),
                misconfiguration: err.is_misconfiguration(),
                retry_after: None,
            })
        }
    }
}

/// Dead-letter a delivery without rescheduling it.
async fn dead_letter(state: &AppState, entry: &EgressOutbox, error: &str) {
    if let Err(e) = state.store.mark_egress_failed(entry.id, error, None).await {
        tracing::warn!(error = %e, id = %entry.id, "egress worker: dead-letter failed");
    }
}

/// Record a transient failure: reschedule with backoff, or dead-letter once the
/// attempts are exhausted. Returns whether it dead-lettered.
async fn record_failure(state: &AppState, entry: &EgressOutbox, failure: &DeliveryFailure) -> bool {
    let error = failure.message.as_str();
    let dead = entry.attempts >= MAX_ATTEMPTS;
    if dead {
        dead_letter(state, entry, error).await;
        tracing::warn!(
            error = %error,
            id = %entry.id,
            attempts = entry.attempts,
            surface = %entry.surface,
            "egress worker: dead-lettered"
        );
        return true;
    }
    let retry_at = chrono::Utc::now()
        + chrono::Duration::from_std(retry_delay(entry.attempts, failure.retry_after))
            .unwrap_or_else(|_| chrono::Duration::seconds(BACKOFF_BASE_SECS as i64));
    if let Err(e) = state
        .store
        .mark_egress_failed(entry.id, error, Some(retry_at))
        .await
    {
        tracing::warn!(error = %e, id = %entry.id, "egress worker: reschedule failed");
    }
    false
}

/// Retry-then-disable: the link is broken in a way no retry fixes, so turn it
/// off, dead-letter this delivery, and say so loudly — a
/// `ProjectorMisconfigured` event, once, on the transition to disabled. Later
/// messages into the link stop enqueueing entirely, so the queue doesn't grind
/// through eight doomed attempts per message; re-linking re-enables it.
async fn disable_link(state: &AppState, entry: &EgressOutbox, target: &EgressTarget, error: &str) {
    dead_letter(state, entry, error).await;
    let disabled = match target {
        EgressTarget::Slack { channel_id, .. } => {
            state.store.disable_slack_channel_link(channel_id).await
        }
        EgressTarget::Github { repo, issue_number } => {
            state
                .store
                .disable_github_issue_link(repo, *issue_number)
                .await
        }
        // No link backs a branch or a Linear target; the dead letter is the
        // whole story.
        EgressTarget::GithubBranch { .. } | EgressTarget::Linear { .. } => return,
    };
    match disabled {
        // Already disabled — another delivery in flight got there first, and the
        // event has already been emitted. Don't announce it twice.
        Ok(false) => return,
        Ok(true) => {}
        Err(err) => {
            tracing::warn!(error = %err, id = %entry.id, "egress worker: disabling the link failed");
            return;
        }
    }
    tracing::warn!(
        %error,
        id = %entry.id,
        surface = %entry.surface,
        selector = %entry.selector,
        "egress worker: projector link disabled (misconfigured)"
    );
    // Best-effort, and resolved the same way the notification router resolves a
    // mention's channel: the event is never withheld for want of context.
    let channel_id = state
        .store
        .get_thread(entry.thread_id)
        .await
        .ok()
        .map(|t| t.channel_id);
    crate::routes::publish(
        state,
        maidan_types::Event::ProjectorMisconfigured {
            occurred_at: chrono::Utc::now(),
            workspace_id: entry.workspace_id,
            channel_id,
            thread_id: entry.thread_id,
            surface: entry.surface.clone(),
            selector: entry.selector.clone(),
            error: error.to_string(),
        },
    )
    .await;
}

/// Persist the handle a result delivery just created/updated, using the row's
/// `armed_revision` as the revision that landed (the two-watermark design).
async fn record_result_landed(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
    reference: Option<&ExternalRef>,
) {
    let Ok(Some(row)) = state
        .store
        .get_result_delivery(entry.thread_id, target)
        .await
    else {
        return;
    };
    let handle = reference.map(|r| r.handle());
    if let Err(err) = state
        .store
        .mark_result_delivered(row.id, handle.as_deref(), row.armed_revision)
        .await
    {
        tracing::warn!(
            error = %err,
            id = %row.id,
            "egress worker: mark-result-delivered failed"
        );
        return;
    }
    crate::metrics::record_result_delivery(target.surface().as_str(), "delivered");
    maidan_observability::observe_stage(
        maidan_observability::ThreadStage::Delivered,
        entry.thread_id,
        Some(entry.workspace_id),
        Some(target.surface().as_str()),
    );
}

/// A change refused for a reason no retry fixes: recorded as a skip, the
/// reason in `last_error`, so the producer and the Slack reply can read it.
async fn record_result_refused(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
    reason: &str,
) {
    let Ok(Some(row)) = state
        .store
        .get_result_delivery(entry.thread_id, target)
        .await
    else {
        return;
    };
    match state
        .store
        .mark_result_delivery_skipped(row.id, reason)
        .await
    {
        Ok(Some(event)) => crate::routes::publish_stored(state, event).await,
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(error = %err, id = %row.id, "egress worker: mark-result-refused failed");
            return;
        }
    }
    tracing::warn!(
        thread_id = %entry.thread_id,
        selector = %entry.selector,
        reason,
        "change flow: refused"
    );
    crate::metrics::record_result_delivery(
        target.surface().as_str(),
        maidan_types::status::SKIPPED,
    );
}

/// The transport gave up. Leave `external_ref` / `delivered_revision` alone —
/// whatever landed before is still out there and still editable.
async fn record_result_gave_up(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
    error: &str,
) {
    let Ok(Some(row)) = state
        .store
        .get_result_delivery(entry.thread_id, target)
        .await
    else {
        return;
    };
    match state.store.mark_result_delivery_failed(row.id, error).await {
        Ok(Some(event)) => crate::routes::publish_stored(state, event).await,
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(
                error = %err,
                id = %row.id,
                "egress worker: mark-result-failed failed"
            );
            return;
        }
    }
    // The event reaches a relay; the operator channel hears it too, naming
    // the thread and where the result was going (#1649), so a stored result
    // nobody received does not wait for someone to notice by hand. A failure
    // no retry fixes lands here on its first try, so the alert says the
    // delivery failed, not that it ran out of retries.
    let reason: String = error.chars().take(200).collect();
    crate::ops_alert::alert(
        &format!("result-delivery-failed:{}:{target}", entry.thread_id),
        &format!(
            "A task result was not delivered: thread {} to {target} failed ({reason}). The result is stored; replay it with POST /threads/{}/deliveries/{}/replay once the cause is fixed.",
            entry.thread_id, entry.thread_id, row.id
        ),
    )
    .await;
    crate::metrics::record_result_delivery(target.surface().as_str(), "failed");
}

/// A send that failed and will be retried. The outbox row carries the
/// retry; the result-delivery row, which is what a producer and the result
/// read see, gets the attempt and the error too, or it would read `pending`,
/// `attempts: 0`, no `last_error` through every retry, as if never tried.
async fn record_result_retry(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
    error: &str,
) {
    let Ok(Some(row)) = state
        .store
        .get_result_delivery(entry.thread_id, target)
        .await
    else {
        return;
    };
    if let Err(err) = state.store.note_result_delivery_retry(row.id, error).await {
        tracing::warn!(error = %err, id = %row.id, "egress worker: noting the retry failed");
    }
    tracing::warn!(
        thread_id = %entry.thread_id,
        surface = %entry.surface,
        selector = %entry.selector,
        attempts = entry.attempts,
        error,
        "egress worker: result delivery failed; will retry"
    );
}

/// Best-effort audit of a result-delivery send attempt. Never fails the
/// delivery itself.
async fn audit_result_attempt(
    state: &AppState,
    entry: &EgressOutbox,
    target: Option<&EgressTarget>,
    outcome: &str,
    error: Option<&str>,
) {
    let target_id = match target {
        Some(t) => state
            .store
            .get_result_delivery(entry.thread_id, t)
            .await
            .ok()
            .flatten()
            .map(|r| r.id.0),
        None => None,
    };
    crate::audit::record(
        state,
        maidan_types::NewAuditEvent {
            scope: maidan_types::AuditScope::Workspace(entry.workspace_id),
            actor_id: None,
            action: "result_delivery.attempt".into(),
            target_kind: Some("result_delivery".into()),
            target_id,
            metadata: serde_json::json!({
                "thread_id": entry.thread_id.0,
                "workspace_id": entry.workspace_id.0,
                "surface": entry.surface,
                "selector": entry.selector,
                "outcome": outcome,
                "error": error,
                "egress_id": entry.id.0,
            }),
        },
    )
    .await;
}

/// The API host a delivery to `target` is posted to, for the retry budget.
fn egress_host(state: &AppState, target: &EgressTarget) -> String {
    match target {
        EgressTarget::Slack { .. } => state.slack_sender.as_ref().map(|s| s.host()),
        EgressTarget::Github { .. } | EgressTarget::GithubBranch { .. } => {
            state.github_sender.as_ref().map(|s| s.host())
        }
        EgressTarget::Linear { .. } => state.linear_sender.as_ref().map(|s| s.host()),
    }
    .unwrap_or_else(|| target.surface().as_str().to_string())
}

/// Drain up to [`MAX_PER_TICK`] due deliveries. No-op when no projector sender is
/// configured — without one, every claim would fail and burn the queue's attempts
/// against a deployment that simply has the projector turned off.
pub async fn sweep_once(state: &AppState) -> EgressSweepStats {
    let mut stats = EgressSweepStats::default();
    if state.slack_sender.is_none()
        && state.github_sender.is_none()
        && state.linear_sender.is_none()
    {
        return stats;
    }
    for _ in 0..MAX_PER_TICK {
        let now = chrono::Utc::now();
        let entry = match state.store.claim_next_due_egress(now, LEASE_SECS).await {
            Ok(Some(e)) => e,
            Ok(None) => break, // queue drained
            Err(err) => {
                tracing::warn!(error = %err, "egress worker: claim failed");
                break;
            }
        };
        // An undecodable destination can never be delivered by any sender, so it
        // dead-letters on the spot instead of burning eight attempts. This is why
        // the claim hands back the stored pair rather than decoding it itself.
        let Some(target) = entry.target() else {
            let error = format!(
                "unroutable destination: {}:{}",
                entry.surface, entry.selector
            );
            dead_letter(state, &entry, &error).await;
            tracing::warn!(id = %entry.id, %error, "egress worker: dead-lettered");
            crate::metrics::record_egress_delivery(
                &entry.surface,
                crate::metrics::EgressOutcome::Unroutable,
            );
            if entry.kind == EgressKind::Result {
                audit_result_attempt(state, &entry, None, "dead", Some(&error)).await;
            }
            stats.dead += 1;
            continue;
        };
        let surface = target.surface().as_str();
        // A result queued for a surface this server has no sender for (armed
        // before the credential was removed, or replayed without it) can
        // never be sent: it fails now, naming the setting, rather than
        // retrying against nothing.
        if entry.kind == EgressKind::Result
            && let Some(reason) =
                crate::result_delivery::unconfigured_reason(state, target.surface())
        {
            dead_letter(state, &entry, &reason).await;
            if let Ok(Some(row)) = state
                .store
                .get_result_delivery(entry.thread_id, &target)
                .await
                && let Err(err) =
                    crate::result_delivery::record_unconfigured(state, &row, &target, &reason).await
            {
                tracing::warn!(error = %err, id = %row.id, "egress worker: mark-result-failed failed");
            }
            audit_result_attempt(state, &entry, Some(&target), "dead", Some(&reason)).await;
            crate::metrics::record_egress_delivery(surface, crate::metrics::EgressOutcome::Dead);
            stats.dead += 1;
            continue;
        }
        // The claim counted this try, so a first post has `attempts == 1`.
        if crate::suspension::held_back(state, entry.workspace_id).await {
            let until = chrono::Utc::now() + crate::suspension::SUSPENDED_DEFER;
            if let Err(err) = state.store.defer_egress(entry.id, until).await {
                tracing::warn!(error = %err, id = %entry.id, "egress worker: deferral failed");
            }
            stats.deferred += 1;
            continue;
        }
        // The hard allowance cutoff: an open sign-up workspace past its
        // deliveries allowance sends nothing. The entry is parked, not
        // failed, and comes back after the cutoff is lifted.
        if state
            .usage_meters
            .is_cut_off(entry.workspace_id, maidan_types::UsageMeter::Deliveries)
        {
            let until = chrono::Utc::now() + crate::usage_meters::CUT_OFF_DEFER;
            if let Err(err) = state.store.defer_egress(entry.id, until).await {
                tracing::warn!(error = %err, id = %entry.id, "egress worker: deferral failed");
            }
            tracing::info!(
                id = %entry.id,
                workspace_id = %entry.workspace_id.0,
                reason = "over_allowance",
                "egress worker: parked; workspace past its free allowance for deliveries"
            );
            crate::metrics::record_egress_delivery(
                surface,
                crate::metrics::EgressOutcome::OverAllowance,
            );
            stats.deferred += 1;
            continue;
        }
        let attempt = Attempt::after(entry.attempts - 1);
        if let Some(until) = deferred_until(
            &state.retry_budget,
            "egress",
            &egress_host(state, &target),
            attempt,
        ) {
            if let Err(err) = state.store.defer_egress(entry.id, until).await {
                // The row stays leased with this claim counted: the one way a
                // deferral can cost an attempt, and only when the store is failing.
                tracing::warn!(error = %err, id = %entry.id, "egress worker: deferral failed");
            }
            stats.deferred += 1;
            continue;
        }
        match maidan_store::trace::maybe_scope(entry.trace.clone(), deliver(state, &entry, &target))
            .await
        {
            Ok(Sent::Waiting) => {
                let until = chrono::Utc::now() + chrono::Duration::seconds(WAITING_RECHECK_SECS);
                if let Err(err) = state.store.defer_egress(entry.id, until).await {
                    tracing::warn!(error = %err, id = %entry.id, "egress worker: deferral failed");
                }
                stats.deferred += 1;
            }
            Ok(Sent::Refused(reason)) => {
                // The transport's part is done; the refusal is the result's.
                if let Err(err) = state.store.mark_egress_delivered(entry.id).await {
                    tracing::warn!(error = %err, id = %entry.id, "egress worker: mark-delivered failed");
                }
                if entry.kind == EgressKind::Result {
                    record_result_refused(state, &entry, &target, &reason).await;
                    audit_result_attempt(state, &entry, Some(&target), "refused", Some(&reason))
                        .await;
                } else {
                    tracing::info!(id = %entry.id, kind = %entry.kind, %reason, "egress worker: refused");
                }
                crate::metrics::record_egress_delivery(
                    surface,
                    crate::metrics::EgressOutcome::Refused,
                );
                stats.refused += 1;
            }
            Ok(Sent::Landed(reference)) => {
                if let Err(err) = state.store.mark_egress_delivered(entry.id).await {
                    tracing::warn!(error = %err, id = %entry.id, "egress worker: mark-delivered failed");
                }
                if entry.kind == EgressKind::Result {
                    record_result_landed(state, &entry, &target, reference.as_ref()).await;
                    audit_result_attempt(state, &entry, Some(&target), "sent", None).await;
                }
                tracing::debug!(
                    id = %entry.id,
                    %surface,
                    kind = %entry.kind,
                    external_ref = reference.as_ref().map(|r| r.handle()),
                    "egress worker: delivered"
                );
                crate::metrics::record_egress_delivery(
                    surface,
                    crate::metrics::EgressOutcome::Sent,
                );
                state.usage_meters.record(
                    entry.workspace_id,
                    maidan_types::UsageMeter::Deliveries,
                    1,
                );
                stats.sent += 1;
            }
            Err(failure) if failure.misconfiguration && entry.kind != EgressKind::Projector => {
                // A result or a notice does not ride a projector issue-link. A
                // 401 here must not disable someone else's linked thread.
                dead_letter(state, &entry, &failure.message).await;
                if entry.kind == EgressKind::Result {
                    record_result_gave_up(state, &entry, &target, &failure.message).await;
                    audit_result_attempt(
                        state,
                        &entry,
                        Some(&target),
                        "dead",
                        Some(&failure.message),
                    )
                    .await;
                }
                crate::metrics::record_egress_delivery(
                    surface,
                    crate::metrics::EgressOutcome::Dead,
                );
                stats.dead += 1;
            }
            Err(failure) if failure.misconfiguration => {
                disable_link(state, &entry, &target, &failure.message).await;
                crate::metrics::record_egress_delivery(
                    surface,
                    crate::metrics::EgressOutcome::Disabled,
                );
                stats.dead += 1;
                stats.disabled += 1;
            }
            Err(failure) => {
                if record_failure(state, &entry, &failure).await {
                    if entry.kind == EgressKind::Result {
                        record_result_gave_up(state, &entry, &target, &failure.message).await;
                        audit_result_attempt(
                            state,
                            &entry,
                            Some(&target),
                            "dead",
                            Some(&failure.message),
                        )
                        .await;
                    }
                    crate::metrics::record_egress_delivery(
                        surface,
                        crate::metrics::EgressOutcome::Dead,
                    );
                    stats.dead += 1;
                } else {
                    if entry.kind == EgressKind::Result {
                        record_result_retry(state, &entry, &target, &failure.message).await;
                        audit_result_attempt(
                            state,
                            &entry,
                            Some(&target),
                            "retry",
                            Some(&failure.message),
                        )
                        .await;
                    }
                    crate::metrics::record_egress_delivery(
                        surface,
                        crate::metrics::EgressOutcome::Retry,
                    );
                    stats.retried += 1;
                }
            }
        }
    }
    stats
}

/// Loop: sweep, then sleep `cfg.tick`. Spawned once at startup when a projector
/// sender is configured.
pub async fn run(state: AppState, cfg: EgressWorkerConfig) {
    tracing::info!(tick_secs = cfg.tick.as_secs(), "egress worker started");
    let _watched = crate::ops_alert::start_loop("egress_worker", cfg.tick);
    loop {
        crate::ops_alert::guarded_tick("egress_worker", sweep_once(&state)).await;
        tokio::time::sleep(cfg.tick).await;
    }
}

/// A round the change flow delivered: its commit, its pull request, and for
/// a squash-rebase that moved the branch, the head it moved from.
struct RoundLanded<'a> {
    commit_sha: &'a str,
    pull_number: i64,
    previous_head: Option<String>,
    linear_issue: Option<String>,
}

/// Record the round for the swarm ramp metrics: the commit, its diff size,
/// and how far its base had fallen behind the base branch. Best effort: a
/// failed read or write never undoes a delivered commit.
async fn record_change_round(
    state: &AppState,
    git: &dyn crate::github::GithubGit,
    entry: &EgressOutbox,
    request: &crate::change_flow::ChangeRequest<'_>,
    landed: RoundLanded<'_>,
) {
    let RoundLanded {
        commit_sha,
        pull_number,
        previous_head,
        linear_issue,
    } = landed;
    let Some(base_sha) = request.change.base_sha.clone() else {
        return;
    };
    // Asked for a rebase round too: one may build on a base commit the base
    // has since moved past, so it is not always current.
    let base_behind = git
        .commits_behind(request.repo, &base_sha, request.base)
        .await
        .ok();
    let round = maidan_types::NewChangeRound {
        workspace_id: entry.workspace_id,
        thread_id: entry.thread_id,
        repo: request.repo.to_ascii_lowercase(),
        pull_number,
        commit_sha: commit_sha.to_string(),
        base_sha,
        base_behind,
        diff_lines: request
            .change
            .diff
            .as_deref()
            .map(maidan_types::diff_lines)
            .unwrap_or(0),
        paths: request
            .change
            .diff
            .as_deref()
            .map(maidan_types::diff_paths)
            .unwrap_or_default(),
        base_branch: Some(request.base.to_string()),
        rebase: request.change.rebase,
        previous_head,
        linear_issue,
    };
    if let Err(err) = state.store.record_change_round(round).await {
        tracing::warn!(repo = request.repo, pull_number, error = %err, "change flow: round not recorded");
    }
    let Some(behind) =
        base_behind.filter(|b| *b >= crate::change_sweep::stale_base_commits_from_env())
    else {
        return;
    };
    let notice = maidan_types::NewStaleBaseNotice {
        thread_id: entry.thread_id,
        repo: request.repo.to_ascii_lowercase(),
        pull_number,
        commit_sha: commit_sha.to_string(),
        base_branch: Some(request.base.to_string()),
        base_behind: behind,
    };
    match state.store.add_stale_base_notice(notice).await {
        Ok(Some(posted)) => crate::change_sweep::post_notice(state, posted).await,
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(repo = request.repo, pull_number, error = %err, "change flow: stale-base notice not recorded");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff_for(1), Duration::from_secs(30));
        assert_eq!(backoff_for(2), Duration::from_secs(60));
        assert_eq!(backoff_for(3), Duration::from_secs(120));
        // Caps at BACKOFF_CAP_SECS and never overflows for large attempt counts.
        assert_eq!(backoff_for(100), Duration::from_secs(BACKOFF_CAP_SECS));
    }

    #[test]
    fn a_retry_waits_out_githubs_retry_after_when_it_is_longer_than_the_backoff() {
        // The first backoff is 30s; GitHub asked for 120s.
        assert_eq!(retry_delay(1, Some(120)), Duration::from_secs(120));
        // Five minutes of backoff already outlasts a 60s window.
        assert_eq!(retry_delay(4, Some(60)), backoff_for(4));
        assert_eq!(retry_delay(1, None), backoff_for(1));
        assert_eq!(retry_delay(1, Some(0)), backoff_for(1));
    }

    #[test]
    fn a_retry_after_past_the_cap_waits_only_the_cap() {
        assert_eq!(
            retry_delay(1, Some(BACKOFF_CAP_SECS * 10)),
            Duration::from_secs(BACKOFF_CAP_SECS)
        );
        assert_eq!(
            retry_delay(1, Some(u64::MAX)),
            Duration::from_secs(BACKOFF_CAP_SECS)
        );
    }

    fn forbidden(rate_limited: bool) -> crate::github::GithubError {
        crate::github::GithubError::Api {
            status: 403,
            rate_limited,
            detail: "Resource not accessible by personal access token".into(),
            retry_after: rate_limited.then_some(60),
        }
    }

    #[test]
    fn a_refused_push_of_a_workflow_file_is_final_and_names_the_file_and_the_fix() {
        let workflows = [".github/workflows/test.yml".to_string()];
        let failure = branch_failure(forbidden(false), &workflows);
        assert!(failure.misconfiguration, "no retry gets a permission back");
        assert_eq!(failure.retry_after, None);
        assert!(
            failure.message.starts_with("github api error: status 403"),
            "the prefix dashboards match on is kept: {}",
            failure.message
        );
        for part in [
            "`.github/workflows/test.yml`",
            "Drop the workflow edit",
            "Workflows permission",
        ] {
            assert!(
                failure.message.contains(part),
                "{part}: {}",
                failure.message
            );
        }
    }

    #[test]
    fn a_rate_limited_403_on_a_workflow_change_still_retries_and_names_the_file() {
        let workflows = [".github/workflows/test.yml".to_string()];
        let failure = branch_failure(forbidden(true), &workflows);
        assert!(!failure.misconfiguration);
        assert_eq!(failure.retry_after, Some(60));
        assert!(failure.message.contains("`.github/workflows/test.yml`"));
        assert!(!failure.message.contains("Workflows permission"));
    }

    #[test]
    fn a_403_on_a_change_without_workflow_files_reads_as_before() {
        let failure = branch_failure(forbidden(false), &[]);
        assert!(failure.misconfiguration);
        assert_eq!(failure.message, forbidden(false).to_string());
        let server_error = crate::github::GithubError::Api {
            status: 500,
            rate_limited: false,
            detail: String::new(),
            retry_after: None,
        };
        let workflows = [".github/workflows/test.yml".to_string()];
        let failure = branch_failure(server_error.clone(), &workflows);
        assert!(!failure.misconfiguration, "a 5xx retries whatever the diff");
        assert_eq!(failure.message, server_error.to_string());
    }

    #[test]
    fn a_rate_limited_github_403_is_a_retry_that_carries_its_retry_after() {
        let failure = DeliveryFailure::github(crate::github::GithubError::Api {
            status: 403,
            rate_limited: true,
            detail: "You have exceeded a secondary rate limit".into(),
            retry_after: Some(90),
        });
        assert!(
            !failure.misconfiguration,
            "a rate limit never disables a link"
        );
        assert_eq!(failure.retry_after, Some(90));
        assert_eq!(
            failure.message,
            "github api error: status 403 (rate limited): You have exceeded a secondary rate limit"
        );
        let denied = DeliveryFailure::github(crate::github::GithubError::Api {
            status: 403,
            rate_limited: false,
            detail: "Resource not accessible by integration".into(),
            retry_after: None,
        });
        assert!(denied.misconfiguration);
        assert_eq!(denied.retry_after, None);
    }
}
