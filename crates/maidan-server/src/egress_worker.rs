//! Background projector-egress worker (Cluster 377.2, durable projector egress).
//!
//! Drains the `maidan_egress_outbox` queue (Cluster 377.1): each tick claims due
//! `pending` deliveries and posts them through the configured projector sender —
//! [`SlackSender`](crate::slack::SlackSender) or
//! [`GithubSender`](crate::github::GithubSender) — marking each delivered, or, on
//! failure, rescheduled with exponential backoff, or dead-lettered once it has
//! exhausted [`MAX_ATTEMPTS`].
//!
//! Replaces the best-effort inline post the Slack (309) and GitHub (312)
//! projectors did, where a transient 502 dropped the message with a log line:
//! `route_message_to_slack` / `route_message_to_github` now only *enqueue*.
//!
//! **Retry-then-disable (Cluster 377.3):** an auth/config-class failure — GitHub
//! 401/403/404, Slack `invalid_auth`/`channel_not_found` — is not retried at all.
//! No number of attempts fixes a revoked token or a deleted channel, so the link
//! is disabled (later messages stop enqueueing), the delivery dead-letters, and a
//! `ProjectorMisconfigured` event names the surface, the selector and the error.
//!
//! **Runs whenever a projector sender is configured** (spawned in `main.rs` only
//! then — and the projectors only enqueue then, so an unconfigured deployment
//! neither queues nor drains). Tick defaults to 5s, tunable via
//! `MAIDAN_EGRESS_WORKER_TICK_SECS`.
//!
//! **At-least-once:** [`claim_next_due_egress`](maidan_store::Store) leases a row
//! forward, so a worker that crashes mid-post releases it after the lease and
//! another claim retries. A duplicate comment is the lesser harm against a
//! silently dropped one — the Cluster-255 digest polarity. Multiple replicas can
//! run the worker safely (`FOR UPDATE SKIP LOCKED` on Postgres hands each a
//! distinct row), and the queue's dedup index means they enqueue one row between
//! them in the first place.
//!
//! **Result delivery (Cluster 379.4):** an outbox row with [`EgressKind::Result`]
//! updates in place when the matching `maidan_result_deliveries` row has an
//! `external_ref`, recovers a GitHub comment via the hidden
//! `<!-- maidan:result:<thread_id> -->` marker if that handle is lost, and
//! otherwise posts. Projector rows (`EgressKind::Projector`) always post — they
//! must not PATCH a result comment that happens to share the issue. A result
//! 401/403/404 dead-letters the delivery without disabling a projector
//! issue-link.
//!
//! **Inline reviews (Cluster 380.2):** after a successful GitHub *summary*
//! comment, the worker POSTs `POST /repos/{repo}/pulls/{n}/reviews` with
//! `commit_id = envelope head_sha` (never the live PR head), `event: COMMENT`,
//! and one inline comment per usable finding (RIGHT, post-image `line_range`).
//! A missing sha, empty findings, a non-`reviewed` status, Slack, or a GitHub
//! 404/422 skips the review; that skip never fails the summary. A 5xx on the
//! review is also a skip — failing the outbox after the summary has already
//! posted would retry into a second issue comment. Replay re-POSTs a review.

use std::time::Duration;

use maidan_types::{EgressKind, EgressOutbox, EgressTarget, ExternalRef, ResultDelivery};

use crate::egress_body::comment_carries_result_marker;
use crate::github::GithubIssueComment;
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

/// Outcome tallies for a sweep (for tests / logging). `disabled` counts the
/// deliveries that dead-lettered because their link was turned off — a subset of
/// the dead-lettered ones, called out because it is the actionable kind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EgressSweepStats {
    pub sent: u32,
    pub retried: u32,
    pub dead: u32,
    pub disabled: u32,
}

/// A failed delivery attempt: what to record, and whether retrying could ever
/// help. A `misconfiguration` is a wrong token, a revoked scope, a channel that
/// no longer exists — no number of retries fixes any of those.
struct DeliveryFailure {
    message: String,
    misconfiguration: bool,
}

/// Post one claimed delivery through the sender for its surface.
///
/// Projector rows always post. Result rows (Cluster 379.4) update in place
/// when a stored [`ExternalRef`] is usable, recover a GitHub comment via the
/// hidden body marker if the ref is lost, and otherwise post. The `Ok`
/// payload is the handle to persist on the result-delivery row.
async fn deliver(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
) -> Result<Option<ExternalRef>, DeliveryFailure> {
    if entry.kind == EgressKind::Result {
        return deliver_result(state, entry, target).await;
    }
    deliver_projector(state, target, &entry.body).await
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
        EgressTarget::Slack { channel_id } => {
            let Some(sender) = state.slack_sender.as_ref() else {
                return Err(DeliveryFailure {
                    message: "no slack sender configured".into(),
                    misconfiguration: false,
                });
            };
            // Top-level: the projector egress relays a Maidan message into the
            // linked channel, and threading those under a parent would change
            // Cluster 309's behaviour.
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
                    })
                }
            }
        }
        EgressTarget::Github { repo, issue_number } => {
            let Some(sender) = state.github_sender.as_ref() else {
                return Err(DeliveryFailure {
                    message: "no github sender configured".into(),
                    misconfiguration: false,
                });
            };
            match sender.post_comment(repo, *issue_number, body).await {
                Ok(reference) => {
                    crate::metrics::record_github_egress("sent");
                    Ok(reference)
                }
                Err(err) => {
                    crate::metrics::record_github_egress("failed");
                    Err(DeliveryFailure {
                        message: err.to_string(),
                        misconfiguration: err.is_misconfiguration(),
                    })
                }
            }
        }
    }
}

async fn deliver_result(
    state: &AppState,
    entry: &EgressOutbox,
    target: &EgressTarget,
) -> Result<Option<ExternalRef>, DeliveryFailure> {
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
            github_result(state, entry, row.as_ref(), repo, *issue_number, &body).await
        }
        EgressTarget::Slack { channel_id } => {
            slack_result(state, row.as_ref(), channel_id, &body).await
        }
    }
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
        });
    };
    let reference =
        deliver_github_result_comment(sender.as_ref(), entry, row, repo, issue_number, body)
            .await?;
    // Additive: a review skip/failure never undoes a landed summary comment.
    post_result_inline_review(state, sender.as_ref(), repo, issue_number, entry.thread_id).await;
    Ok(reference)
}

/// The Cluster 379 summary comment: update in place, recover via the marker,
/// or post. Returns the handle to persist; does not post inline findings.
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
                return Err(DeliveryFailure {
                    message: err.to_string(),
                    misconfiguration: err.is_misconfiguration(),
                });
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
                    return Err(DeliveryFailure {
                        message: err.to_string(),
                        misconfiguration: err.is_misconfiguration(),
                    });
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
            Err(DeliveryFailure {
                message: err.to_string(),
                misconfiguration: err.is_misconfiguration(),
            })
        }
    }
}

/// Cluster 380.2: post a COMMENT review for the current waiter envelope.
///
/// Never returns an error. The summary comment has already landed; failing
/// the outbox here would retry into a duplicate issue comment on a first
/// delivery. 404 (issue is not a PR) and 422 (line not in the diff at
/// `head_sha`) are skips. A 5xx is logged and left for operator replay,
/// which PATCHes the summary and POSTs another COMMENT review.
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
            // 404/422/401/403/5xx: the summary stays. Replay retries the review.
            crate::metrics::record_github_review("failed");
            tracing::warn!(
                error = %err,
                %repo,
                pull = issue_number,
                commit_id = %review.commit_id,
                "github inline review: not posted; summary comment still delivered"
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
            Err(DeliveryFailure {
                message: err.to_string(),
                misconfiguration: err.is_misconfiguration(),
            })
        }
    }
}

async fn slack_result(
    state: &AppState,
    row: Option<&ResultDelivery>,
    channel_id: &str,
    body: &str,
) -> Result<Option<ExternalRef>, DeliveryFailure> {
    let Some(sender) = state.slack_sender.as_ref() else {
        return Err(DeliveryFailure {
            message: "no slack sender configured".into(),
            misconfiguration: false,
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
                });
            }
        }
    }
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
async fn record_failure(state: &AppState, entry: &EgressOutbox, error: &str) -> bool {
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
        + chrono::Duration::from_std(backoff_for(entry.attempts))
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

/// Retry-then-disable (Cluster 377.3): the link is broken in a way no retry
/// fixes, so turn it off, dead-letter this delivery, and say so loudly — a
/// `ProjectorMisconfigured` event, once, on the transition to disabled. Later
/// messages into the link stop enqueueing entirely, so the queue doesn't grind
/// through eight doomed attempts per message; re-linking re-enables it.
async fn disable_link(state: &AppState, entry: &EgressOutbox, target: &EgressTarget, error: &str) {
    dead_letter(state, entry, error).await;
    let disabled = match target {
        EgressTarget::Slack { channel_id } => state.store.disable_slack_channel_link(channel_id),
        EgressTarget::Github { repo, issue_number } => {
            state.store.disable_github_issue_link(repo, *issue_number)
        }
    }
    .await;
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
    crate::metrics::record_result_delivery("delivered");
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
    if let Err(err) = state.store.mark_result_delivery_failed(row.id, error).await {
        tracing::warn!(
            error = %err,
            id = %row.id,
            "egress worker: mark-result-failed failed"
        );
        return;
    }
    crate::metrics::record_result_delivery("failed");
}

/// Best-effort audit of a result-delivery send attempt (Cluster 379.5).
/// Never fails the delivery itself.
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

/// Drain up to [`MAX_PER_TICK`] due deliveries. No-op when no projector sender is
/// configured — without one, every claim would fail and burn the queue's attempts
/// against a deployment that simply has the projector turned off.
pub async fn sweep_once(state: &AppState) -> EgressSweepStats {
    let mut stats = EgressSweepStats::default();
    if state.slack_sender.is_none() && state.github_sender.is_none() {
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
            crate::metrics::record_egress_delivery(&entry.surface, "unroutable");
            if entry.kind == EgressKind::Result {
                audit_result_attempt(state, &entry, None, "dead", Some(&error)).await;
            }
            stats.dead += 1;
            continue;
        };
        let surface = target.surface().as_str();
        match deliver(state, &entry, &target).await {
            Ok(reference) => {
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
                crate::metrics::record_egress_delivery(surface, "sent");
                stats.sent += 1;
            }
            Err(failure) if failure.misconfiguration && entry.kind == EgressKind::Result => {
                // A result delivery does not ride a projector issue-link. A 401
                // here must not disable someone else's linked thread.
                dead_letter(state, &entry, &failure.message).await;
                record_result_gave_up(state, &entry, &target, &failure.message).await;
                audit_result_attempt(state, &entry, Some(&target), "dead", Some(&failure.message))
                    .await;
                crate::metrics::record_egress_delivery(surface, "dead");
                stats.dead += 1;
            }
            Err(failure) if failure.misconfiguration => {
                disable_link(state, &entry, &target, &failure.message).await;
                crate::metrics::record_egress_delivery(surface, "disabled");
                stats.dead += 1;
                stats.disabled += 1;
            }
            Err(failure) => {
                if record_failure(state, &entry, &failure.message).await {
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
                    crate::metrics::record_egress_delivery(surface, "dead");
                    stats.dead += 1;
                } else {
                    if entry.kind == EgressKind::Result {
                        audit_result_attempt(
                            state,
                            &entry,
                            Some(&target),
                            "retry",
                            Some(&failure.message),
                        )
                        .await;
                    }
                    crate::metrics::record_egress_delivery(surface, "retry");
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
    loop {
        sweep_once(&state).await;
        tokio::time::sleep(cfg.tick).await;
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
}
