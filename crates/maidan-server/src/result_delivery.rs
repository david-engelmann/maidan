//! Result delivery trigger.
//!
//! A `ThreadResultSet` is a "go fetch" pointer: the envelope lives on the
//! thread, not on the event. This module is the arm of
//! [`crate::notification_router::route_event`] that fetches, parses, and — per
//! `deliver_to` target — allowlist-checks then enqueues. It is the exact shape
//! `MessagePosted → route_message_to_slack` already has, pointed at a result
//! instead of a projector-linked channel.
//!
//! **`deliver_to` selects; the workspace allowlist authorizes.** An empty
//! `deliver_to` is valid and normal (thread-only, zero rows). A target the
//! workspace has not blessed, or a surface this build does not know, is a
//! *skip with a recorded warning* — never silence, never an error that sinks
//! the other targets. Partial delivery is the model.
//!
//! The every-replica router is safe because `arm_result_delivery` is the
//! contended write: exactly one replica wins the right to enqueue. The egress
//! outbox is *transport* (retry/backoff); this module only arms intent and, for
//! a blessed target, puts an [`EgressKind::Result`] row on that queue. The
//! worker updates in place via the stored `external_ref`, recovering a GitHub
//! comment through the hidden body marker if the handle is lost.
//!
//! After a successful GitHub summary comment, the worker
//! POSTs a `COMMENT` review whose `commit_id` is envelope `head_sha` and whose
//! inline comments are the usable findings (RIGHT, post-image `line_range`). A
//! missing sha, empty findings, a non-`reviewed` status, a vanished envelope,
//! or a GitHub 404/422 skips the review; a 5xx is left for operator replay. The
//! 379 summary path is unchanged.

use chrono::{DateTime, Utc};
use maidan_types::{
    parse_change_result, parse_waiter_result, status, DeliverTarget, EgressKind, EgressTarget,
    ExternalRef, GithubReviewComment, NewEgressOutbox, ResultAttachment, ResultDelivery, ThreadId,
    WaiterResult, WorkspaceId, CHANGE_RESULT_KIND,
};
use tracing::{debug, warn};

use crate::egress_body::{
    github_result_comment_body, github_review_comment_body, slack_message_body,
};
use crate::github::GITHUB_REVIEW_COMMENTS_MAX;
use crate::state::AppState;

/// The short Maidan-authored notice a non-`reviewed` result delivers. Built
/// from `status` alone — never the producer's `rendered` or `summary`, so a
/// failed review can never look like a clean pass, and is never silent.
pub fn failure_notice(status: &str) -> String {
    format!(
        "Maidan could not deliver this result: the producer reported status `{status}`, not `reviewed`."
    )
}

/// The body that will actually leave Maidan for this target.
///
/// GitHub gets `rendered` (GFM, mentions defused, truncated to the comment
/// ceiling) with a hidden `<!-- maidan:result:<thread_id> -->` marker at byte 0
/// — the recovery path if the stored `external_ref` is lost.
/// Slack gets `summary` plus a compact digest — **never** `rendered`, which is
/// GFM and would arrive visibly broken. A non-`reviewed` status replaces both
/// with [`failure_notice`].
///
/// **Every Slack body is escaped on the way out**, including the
/// failure notice. The notice interpolates the producer's `status`, which is an
/// arbitrary agent-written string, and it used to be returned raw: a result
/// with `status: "<!channel>"` aimed at a blessed channel was a real broadcast
/// under Maidan's identity. Escaping is idempotent, so the reviewed path —
/// already escaped inside `slack_message_body` — is unchanged by passing
/// through again.
pub fn delivery_body(
    thread_id: ThreadId,
    target: &EgressTarget,
    waiter: &WaiterResult,
    public_origin: Option<&str>,
) -> String {
    let inner = if waiter.is_reviewed() {
        reviewed_body(target, waiter, public_origin)
    } else {
        // Still marked on GitHub so a later reviewed result updates this
        // comment rather than stacking a second one.
        failure_notice(&waiter.status)
    };
    match target {
        EgressTarget::Github { .. } => {
            let backlink = waiter
                .is_reviewed()
                .then_some(waiter.view_url.as_deref())
                .flatten();
            // Failure notices are already the complete inner body; reviewed
            // inner is the (PR-annotated) rendered. The helper defuses and
            // truncates either way, reserving the marker in the budget.
            github_result_comment_body(thread_id, &inner, backlink)
        }
        EgressTarget::Slack { .. } => slack_body(&inner),
        EgressTarget::Linear { .. } => crate::egress_body::linear_result_comment_body(
            &inner,
            waiter
                .is_reviewed()
                .then_some(waiter.view_url.as_deref())
                .flatten(),
        ),
        // The commit and pull request are built from the envelope at send
        // time; this is only what the outbox row says it carries.
        EgressTarget::GithubBranch { repo, branch } => {
            format!("{} for {repo}@{branch}", waiter.result_kind)
        }
    }
}

/// One line per attached file, under a header that says the files need
/// Maidan auth: `/artifacts/{sha}` sits behind the artifact routes' member
/// check, so a reader without a Maidan sign-in reaches a login wall, not
/// the file. With a configured public origin the lines are absolute links
/// — a root-relative link inside a GitHub comment resolves against
/// github.com and 404s, so the origin is what makes the link open the
/// file at all. Without one the files are named, not linked; the reader
/// opens them from the thread in Maidan. The label is the producer's
/// caption, or the filename when there is no caption — escaped either
/// way, because the caption is producer text and the filename is about
/// to become link text.
fn attachment_lines(attachments: &[ResultAttachment], public_origin: Option<&str>) -> String {
    let mut out = match public_origin {
        Some(_) => String::from("Attachments (the links open in Maidan and need Maidan auth):"),
        None => String::from(
            "Attachments (open them from this thread in Maidan; they need Maidan auth):",
        ),
    };
    for attachment in attachments {
        let label = attachment
            .caption
            .as_deref()
            .unwrap_or(attachment.filename.as_str());
        let label = escape_markdown_label(label);
        match public_origin {
            Some(origin) => out.push_str(&format!(
                "\n- [{}]({}/artifacts/{})",
                label,
                origin.trim_end_matches('/'),
                attachment.sha
            )),
            None => out.push_str(&format!("\n- {label}")),
        }
    }
    out
}

/// Backslash-escape the characters that would let producer text style
/// itself or break out of a Markdown link label. The label is data, never
/// markup.
fn escape_markdown_label(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(
            ch,
            '\\' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '#' | '<' | '>' | '|' | '~' | '!'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn slack_body(text: &str) -> String {
    crate::egress_body::truncate_with_tail(
        &crate::egress_body::neutralize_slack_mentions(text),
        crate::egress_body::SLACK_BODY_MAX_CHARS,
        None,
    )
}

fn reviewed_body(
    target: &EgressTarget,
    waiter: &WaiterResult,
    public_origin: Option<&str>,
) -> String {
    let backlink = waiter.view_url.as_deref();
    match target {
        EgressTarget::Github { .. } | EgressTarget::Linear { .. } => {
            let mut rendered = waiter.rendered.clone().unwrap_or_default();
            if let Some(pr) = &waiter.pr {
                if !rendered.is_empty() {
                    rendered.push_str("\n\n");
                }
                rendered.push_str("PR: ");
                rendered.push_str(pr);
            }
            if !waiter.attachments.is_empty() {
                if !rendered.is_empty() {
                    rendered.push_str("\n\n");
                }
                rendered.push_str(&attachment_lines(&waiter.attachments, public_origin));
            }
            // Mentions / truncation happen in `github_result_comment_body`
            // so the marker is reserved in the ceiling. Pass the raw
            // rendered here; defanging twice would wrap `@x` as `` `@x` ``.
            rendered
        }
        EgressTarget::Slack { .. } => {
            let summary = waiter
                .summary
                .as_deref()
                .unwrap_or(waiter.result_kind.as_str());
            let digest = waiter
                .pr
                .as_deref()
                .map(|pr| vec![format!("PR: {pr}")])
                .unwrap_or_default();
            slack_message_body(summary, &digest, backlink)
        }
        EgressTarget::GithubBranch { .. } => String::new(),
    }
}

/// Fetch the thread's result, parse the waiter envelope, and per target either
/// skip (recorded) or enqueue onto the egress outbox.
///
/// `None` from the parser is inert — an unrecognized `schema` means no
/// delivery is attempted, because routing on an envelope we do not understand
/// is how you deliver the wrong bytes to the wrong place. An empty
/// `deliver_to` returns without writing a row: that is a supported outcome,
/// not a misconfiguration.
pub async fn route_thread_result(
    state: &AppState,
    log_id: i64,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
) -> Result<(), String> {
    let stored = match state.store.get_thread_result(thread_id).await {
        Ok(Some(stored)) => stored,
        Ok(None) => {
            debug!(%thread_id, "result delivery: no result on the thread");
            return Ok(());
        }
        Err(err) => return Err(err.to_string()),
    };
    let Some(waiter) = parse_waiter_result(&stored.result) else {
        debug!(
            %thread_id,
            "result delivery: unrecognized envelope, not delivering"
        );
        return Ok(());
    };
    if waiter.deliver_to.is_empty() {
        return Ok(());
    }
    // Branch targets arm first: a change's Slack reply reads their rows, so
    // they must exist before the reply can be claimed.
    let (branches, others): (Vec<&DeliverTarget>, Vec<&DeliverTarget>) = waiter
        .deliver_to
        .iter()
        .partition(|t| matches!(t, DeliverTarget::GithubBranch { .. }));
    for target in branches.into_iter().chain(others) {
        if let Err(err) = route_one(
            state,
            log_id,
            workspace_id,
            thread_id,
            stored.produced_at,
            &waiter,
            target,
        )
        .await
        {
            // Partial delivery: one target failing must not sink the others.
            warn!(
                error = %err,
                %thread_id,
                surface = target.surface(),
                "result delivery: target failed; continuing"
            );
        }
    }
    Ok(())
}

/// Feed a delivered (or thread-only) `example.review.result/1` with any
/// `critical` finding into the close-gate. Uses the result's `produced_by` as
/// the reviewer — they must have declared the `review` skill. Empty
/// `deliver_to` still arms: the room blocks the land even when nothing is
/// posted externally. Idempotent (upsert + `k` only when unset) so every
/// replica of the router can call it.
pub async fn arm_critical_review(state: &AppState, thread_id: ThreadId) -> Result<(), String> {
    let stored = match state.store.get_thread_result(thread_id).await {
        Ok(Some(stored)) => stored,
        Ok(None) => return Ok(()),
        Err(err) => return Err(err.to_string()),
    };
    match state
        .store
        .apply_critical_review_decision(thread_id, stored.produced_by, &stored.result)
        .await
    {
        Ok(Some(submission)) => {
            for stored in submission.events() {
                crate::routes::publish_stored(state, stored.clone()).await;
            }
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

async fn route_one(
    state: &AppState,
    log_id: i64,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
    revision: DateTime<Utc>,
    waiter: &WaiterResult,
    target: &DeliverTarget,
) -> Result<(), String> {
    match target.to_egress_target() {
        None => {
            let (surface, selector) = target.skip_fingerprint();
            let reason = if matches!(target, DeliverTarget::Unknown(_)) {
                format!("unknown surface '{surface}'")
            } else {
                format!("unusable {surface} target")
            };
            skip_unroutable(state, thread_id, &surface, &selector, revision, &reason).await
        }
        Some(egress) => {
            // The hard-coded branch rules come before the allowlist: no
            // blessing can make `main` or a `prod` base writable.
            if let DeliverTarget::GithubBranch { branch, base, .. } = target
                && let Err(reason) = maidan_types::check_change_target(branch, base)
            {
                return skip_routable(state, thread_id, &egress, revision, &reason).await;
            }
            let selector = target
                .allowlist_selector()
                .unwrap_or_else(|| egress.allowlist_selector());
            let allowed = state
                .store
                .is_egress_target_allowed(workspace_id, egress.surface(), &selector)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(reason) = allowed.then(|| no_change_reason(&egress, waiter)).flatten() {
                skip_routable(state, thread_id, &egress, revision, &reason).await
            } else if allowed {
                enqueue_routable(
                    state,
                    log_id,
                    workspace_id,
                    thread_id,
                    revision,
                    waiter,
                    &egress,
                )
                .await
            } else {
                skip_routable(
                    state,
                    thread_id,
                    &egress,
                    revision,
                    "target not in the workspace egress allowlist",
                )
                .await
            }
        }
    }
}

/// Why a blessed `github_branch` target still gets no GitHub write: only a
/// [`CHANGE_RESULT_KIND`] with status `changed` is committed.
fn no_change_reason(target: &EgressTarget, waiter: &WaiterResult) -> Option<String> {
    if !matches!(target, EgressTarget::GithubBranch { .. }) {
        return None;
    }
    if waiter.result_kind != CHANGE_RESULT_KIND {
        return Some(format!(
            "result kind `{}` carries no change to commit",
            waiter.result_kind
        ));
    }
    if waiter.status == maidan_types::CHANGE_STATUS_REBASE_CONFLICT {
        return Some(format!(
            "status `{}`: the rebase stopped on conflicts, so no GitHub write",
            waiter.status
        ));
    }
    (waiter.status != maidan_types::CHANGE_STATUS_CHANGED).then(|| {
        format!(
            "status `{}`: nothing to commit, so no GitHub write",
            waiter.status
        )
    })
}

/// The reply to a change result, which reports what happened on GitHub. A
/// Slack thread and a Linear issue both get it.
pub enum ChangeReply {
    /// Not a change result: the ordinary body applies.
    NotAChange,
    /// A branch delivery for this result has not finished yet.
    Waiting,
    /// The reply as plain text; each surface escapes it its own way
    /// ([`slack_reply_body`], [`linear_reply_body`]).
    Ready(String),
}

/// A change reply as a Slack message.
pub fn slack_reply_body(reply: &str) -> String {
    slack_body(reply)
}

/// A change reply as a Linear comment.
pub fn linear_reply_body(reply: &str) -> String {
    crate::egress_body::linear_result_comment_body(reply, None)
}

/// How long a change's Slack reply waits for its branch delivery. The branch's
/// own retries give up within about two hours (eight attempts, backoff capped
/// at an hour); past this the reply says the commit has not landed rather
/// than waiting on a row that may never settle.
pub const CHANGE_REPLY_MAX_WAIT: chrono::Duration = chrono::Duration::hours(6);

/// The longest a change reply's reason runs, in characters.
const CHANGE_REASON_MAX_CHARS: usize = 600;

/// A run's failure reason, short enough for a chat line: a cause repeated
/// across retries is said once (lines are compared trimmed, first occurrence
/// kept), and the result is cut at [`CHANGE_REASON_MAX_CHARS`] on a character
/// boundary with an ellipsis.
fn compact_reason(reason: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let kept: Vec<&str> = reason
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && seen.insert(*line))
        .collect();
    let joined = kept.join("\n");
    if joined.chars().count() <= CHANGE_REASON_MAX_CHARS {
        return joined;
    }
    let mut cut: String = joined.chars().take(CHANGE_REASON_MAX_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// Build the reply to a [`CHANGE_RESULT_KIND`] from its
/// `github_branch` deliveries: the commit and pull request when one landed,
/// the recorded reason when it did not, and the status alone when the producer reported
/// anything other than `changed`.
pub async fn change_reply(
    state: &AppState,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
) -> ChangeReply {
    change_reply_at(state, workspace_id, thread_id, Utc::now()).await
}

/// [`change_reply`] as of `now`, which bounds the wait.
pub async fn change_reply_at(
    state: &AppState,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
    now: DateTime<Utc>,
) -> ChangeReply {
    let Ok(Some(stored)) = state.store.get_thread_result(thread_id).await else {
        return ChangeReply::NotAChange;
    };
    let (Some(waiter), Some(change)) = (
        parse_waiter_result(&stored.result),
        parse_change_result(&stored.result),
    ) else {
        return ChangeReply::NotAChange;
    };
    let mut lines = Vec::new();
    if !change.is_changed() {
        lines.push(if change.status == maidan_types::CHANGE_STATUS_REBASE_CONFLICT {
            format!(
                "The coding agent's rebase onto the base branch stopped on conflicts (status `{}`), so nothing was committed.",
                change.status
            )
        } else {
            format!(
                "The coding agent finished with status `{}`, so nothing was committed.",
                change.status
            )
        });
        // The cause, so the thread gets one end notice that says why. A
        // producer's own detail wins over its summary, as the consuming app reads it.
        if let Some(reason) = change.error_detail.as_deref().or(change.summary.as_deref()) {
            lines.push(compact_reason(reason));
        }
        return ChangeReply::Ready(lines.join("\n"));
    }
    let gave_up = now - stored.produced_at > CHANGE_REPLY_MAX_WAIT;
    let unsettled = |repo: &str, branch: &str| {
        format!(
            "Nothing has landed on `{branch}` in {repo} after {} hours; the delivery status has the latest.",
            CHANGE_REPLY_MAX_WAIT.num_hours()
        )
    };
    for target in &waiter.deliver_to {
        let DeliverTarget::GithubBranch { repo, branch, .. } = target else {
            continue;
        };
        let Some(egress) = target.to_egress_target() else {
            lines.push(format!(
                "Nothing was committed to `{branch}` in {repo}: the target is not usable."
            ));
            continue;
        };
        let row = match state.store.get_result_delivery(thread_id, &egress).await {
            Ok(Some(row)) if row.armed_revision >= stored.produced_at => row,
            Ok(_) => {
                lines.push(format!(
                    "Nothing was committed to `{branch}` in {repo}: the delivery was not recorded."
                ));
                continue;
            }
            Err(_) if gave_up => {
                lines.push(unsettled(repo, branch));
                continue;
            }
            Err(_) => return ChangeReply::Waiting,
        };
        match (row.status.as_str(), row.reference()) {
            (status::PENDING, _) if gave_up => lines.push(unsettled(repo, branch)),
            (status::PENDING, _) => return ChangeReply::Waiting,
            (
                status::DELIVERED,
                Some(ExternalRef::GithubBranch {
                    commit_sha,
                    pull_number,
                    ..
                }),
            ) => {
                lines.push(format!(
                    "Committed {commit_sha} to `{branch}` in {repo}. Draft PR: https://github.com/{repo}/pull/{pull_number}"
                ));
                // The same range the pull request was told, from the round
                // the delivery recorded. A failed read only drops the line.
                if let (true, Some(base_sha)) = (change.rebase, change.base_sha.as_deref())
                    && let Ok(Some(old_head)) = state
                        .store
                        .change_round_previous_head(workspace_id, repo, pull_number, &commit_sha)
                        .await
                {
                    lines.push(crate::change_flow::rebase_note(
                        &old_head,
                        &commit_sha,
                        base_sha,
                    ));
                }
                lines.extend(crate::change_flow::template_note(&row.template_warnings));
            }
            _ => lines.push(format!(
                "Nothing was committed to `{branch}` in {repo}: {}",
                row.last_error
                    .as_deref()
                    .unwrap_or("the delivery did not land")
            )),
        }
    }
    if lines.is_empty() {
        lines.push("The result named no `github_branch` target, so nothing was committed.".into());
    }
    ChangeReply::Ready(lines.join("\n"))
}

async fn skip_routable(
    state: &AppState,
    thread_id: ThreadId,
    target: &EgressTarget,
    revision: DateTime<Utc>,
    reason: &str,
) -> Result<(), String> {
    let Some(row) = state
        .store
        .arm_result_delivery(thread_id, target, revision)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    record_skip(state, row, reason).await
}

async fn skip_unroutable(
    state: &AppState,
    thread_id: ThreadId,
    surface: &str,
    selector: &str,
    revision: DateTime<Utc>,
    reason: &str,
) -> Result<(), String> {
    let Some(row) = state
        .store
        .arm_unroutable_result_delivery(thread_id, surface, selector, revision)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    record_skip(state, row, reason).await
}

async fn record_skip(state: &AppState, row: ResultDelivery, reason: &str) -> Result<(), String> {
    if let Some(event) = state
        .store
        .mark_result_delivery_skipped(row.id, reason)
        .await
        .map_err(|e| e.to_string())?
    {
        crate::routes::publish_stored(state, event).await;
    }
    crate::metrics::record_result_delivery(status::SKIPPED);
    warn!(
        thread_id = %row.thread_id,
        surface = %row.surface,
        selector = %row.selector,
        reason,
        "result delivery: skipped"
    );
    Ok(())
}

async fn enqueue_routable(
    state: &AppState,
    log_id: i64,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
    revision: DateTime<Utc>,
    waiter: &WaiterResult,
    target: &EgressTarget,
) -> Result<(), String> {
    let Some(_row) = state
        .store
        .arm_result_delivery(thread_id, target, revision)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    let body = delivery_body(thread_id, target, waiter, state.public_origin.as_deref());
    state
        .store
        .enqueue_egress(NewEgressOutbox {
            workspace_id,
            thread_id,
            source_log_id: log_id,
            target: target.clone(),
            body,
            kind: EgressKind::Result,
        })
        .await
        .map_err(|e| e.to_string())?;
    crate::metrics::record_result_delivery("enqueued");
    Ok(())
}

/// Rebuild the body the worker should send for this target from the thread's
/// current result. `None` when there is no waiter envelope to render, so the
/// caller falls back to the outbox snapshot (a replay of a deleted result
/// still delivers what was queued).
pub async fn current_delivery_body(
    state: &AppState,
    thread_id: ThreadId,
    target: &maidan_types::EgressTarget,
) -> Option<String> {
    let waiter = current_waiter(state, thread_id).await?;
    Some(delivery_body(
        thread_id,
        target,
        &waiter,
        state.public_origin.as_deref(),
    ))
}

/// The live waiter envelope, if the thread still has one Maidan recognizes.
pub async fn current_waiter(state: &AppState, thread_id: ThreadId) -> Option<WaiterResult> {
    let stored = state.store.get_thread_result(thread_id).await.ok()??;
    parse_waiter_result(&stored.result)
}

/// Coordinates for one `POST /repos/{repo}/pulls/{n}/reviews`. `None` means
/// skip the inline review: the 379 summary comment still posts.
pub struct PreparedInlineReview {
    pub commit_id: String,
    pub comments: Vec<GithubReviewComment>,
    pub truncated: bool,
}

/// Map a waiter envelope onto a GitHub review payload.
///
/// `None` when the result is not `reviewed`, has no envelope `head_sha`, or
/// has no usable findings. Mentions in finding bodies are defused here; the
/// 379 recovery marker is **not** applied. Findings past GitHub's 100-comment
/// cap are dropped (`truncated`).
///
/// `commit_id` is **only** [`WaiterResult::review_commit_id`] — there is no
/// live-PR-head lookup, and there must not be.
pub fn prepare_inline_review(waiter: &WaiterResult) -> Option<PreparedInlineReview> {
    if !waiter.is_reviewed() {
        return None;
    }
    let commit_id = waiter.review_commit_id()?.to_string();
    let mut comments: Vec<GithubReviewComment> = waiter
        .github_review_comments()
        .into_iter()
        .filter_map(|comment| {
            let body = github_review_comment_body(&comment.body);
            if body.is_empty() {
                return None;
            }
            Some(GithubReviewComment { body, ..comment })
        })
        .collect();
    if comments.is_empty() {
        return None;
    }
    let truncated = comments.len() > GITHUB_REVIEW_COMMENTS_MAX;
    if truncated {
        comments.truncate(GITHUB_REVIEW_COMMENTS_MAX);
    }
    Some(PreparedInlineReview {
        commit_id,
        comments,
        truncated,
    })
}

/// Best-effort audit of an operator replay. Never fails the replay itself.
pub async fn audit_replay(
    state: &AppState,
    workspace_id: maidan_types::WorkspaceId,
    actor_id: Option<maidan_types::MemberId>,
    row: &ResultDelivery,
) {
    crate::audit::record(
        state,
        maidan_types::NewAuditEvent {
            scope: maidan_types::AuditScope::Workspace(workspace_id),
            actor_id,
            action: "result_delivery.replay".into(),
            target_kind: Some("result_delivery".into()),
            target_id: Some(row.id.0),
            metadata: serde_json::json!({
                "thread_id": row.thread_id.0,
                "surface": row.surface,
                "selector": row.selector,
                "status": row.status,
            }),
        },
    )
    .await;
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_change_reason_says_a_repeated_cause_once() {
        let reason = "pi gave up after 3 failed attempts\nseat timed out\n  seat timed out\n\nseat timed out";
        assert_eq!(
            super::compact_reason(reason),
            "pi gave up after 3 failed attempts\nseat timed out"
        );
    }

    #[test]
    fn a_long_change_reason_is_cut_on_a_character_boundary() {
        let reason = "é".repeat(super::CHANGE_REASON_MAX_CHARS + 50);
        let cut = super::compact_reason(&reason);
        assert_eq!(cut.chars().count(), super::CHANGE_REASON_MAX_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(super::compact_reason("short"), "short");
    }

    use super::*;
    use maidan_types::{
        FindingLineRange, GithubDiffSide, WaiterFinding, STATUS_REVIEWED, WAITER_RESULT_SCHEMA,
    };

    fn waiter(status: &str, rendered: Option<&str>, summary: Option<&str>) -> WaiterResult {
        WaiterResult {
            result_kind: "example.review.result/1".into(),
            status: status.into(),
            deliver_to: vec![],
            rendered: rendered.map(str::to_string),
            summary: summary.map(str::to_string),
            view_url: Some("https://producer.example.test/r/1".into()),
            pr: Some("acme/widgets#7".into()),
            head_sha: None,
            findings: vec![],
            attachments: vec![],
        }
    }

    fn github() -> EgressTarget {
        EgressTarget::Github {
            repo: "acme/widgets".into(),
            issue_number: 7,
        }
    }

    fn slack() -> EgressTarget {
        EgressTarget::Slack {
            channel_id: "C0123ABCDEF".into(),
            thread_ts: None,
        }
    }

    #[test]
    fn a_non_reviewed_body_is_maidan_authored_from_status_alone() {
        let waiter = waiter(
            "failed",
            Some("looks like a clean pass @octocat"),
            Some("<!channel> ship it"),
        );
        let tid = ThreadId::new();
        for target in [github(), slack()] {
            let body = delivery_body(tid, &target, &waiter, None);
            assert!(
                body.contains(&failure_notice("failed")),
                "the notice must still be the body: {body}"
            );
            assert!(
                !body.contains("clean pass") && !body.contains("@octocat"),
                "the producer's rendered must never ride a non-reviewed delivery: {body}"
            );
            assert!(
                !body.contains("ship it") && !body.contains("<!channel>"),
                "the producer's summary must never ride a non-reviewed delivery: {body}"
            );
        }
        let gh = delivery_body(tid, &github(), &waiter, None);
        assert!(
            crate::egress_body::comment_carries_result_marker(&gh, tid),
            "a failure notice on GitHub is still marked so a later review updates it: {gh}"
        );
        let sl = delivery_body(tid, &slack(), &waiter, None);
        assert_eq!(sl, failure_notice("failed"));
    }

    #[test]
    fn github_gets_rendered_and_slack_gets_summary() {
        let waiter = waiter(
            STATUS_REVIEWED,
            Some("## Findings\n\nping @octocat"),
            Some("3 findings"),
        );
        let tid = ThreadId::new();
        let gh = delivery_body(tid, &github(), &waiter, None);
        assert!(
            crate::egress_body::comment_carries_result_marker(&gh, tid),
            "the recovery marker is at byte 0: {gh:.80}"
        );
        assert!(gh.contains("Findings"), "github delivers rendered GFM");
        assert!(
            gh.contains("`@octocat`"),
            "mentions are defused at the egress boundary: {gh}"
        );
        assert!(
            !gh.contains("3 findings"),
            "github does not substitute the slack summary"
        );
        assert!(gh.contains("PR: acme/widgets#7"));
        assert!(gh.contains("https://producer.example.test/r/1"));

        let sl = delivery_body(tid, &slack(), &waiter, None);
        assert!(sl.contains("3 findings"), "slack delivers the summary");
        assert!(
            !sl.contains("Findings") && !sl.contains("@octocat"),
            "slack must never receive rendered GFM: {sl}"
        );
        assert!(sl.contains("PR: acme/widgets#7"));
        assert!(sl.contains("https://producer.example.test/r/1"));
        assert!(
            !sl.contains("<!-- maidan:result:"),
            "slack has no HTML-comment recovery marker"
        );
    }

    fn attachment(sha: &str, filename: &str, caption: Option<&str>) -> ResultAttachment {
        ResultAttachment {
            sha: sha.into(),
            filename: filename.into(),
            caption: caption.map(str::to_string),
        }
    }

    #[test]
    fn github_appends_one_line_per_attachment_and_slack_gets_none() {
        let mut waiter = waiter(STATUS_REVIEWED, Some("## Findings"), Some("3 findings"));
        waiter.attachments = vec![
            attachment(&"a".repeat(64), "trace.png", Some("The failing run")),
            attachment(&"b".repeat(64), "notes.md", None),
        ];
        let tid = ThreadId::new();
        let gh = delivery_body(tid, &github(), &waiter, Some("https://maidan.example.com"));
        assert!(
            gh.contains("Attachments (the links open in Maidan and need Maidan auth):"),
            "the header says the links need Maidan auth: {gh}"
        );
        assert!(
            gh.contains(&format!(
                "- [The failing run](https://maidan.example.com/artifacts/{})",
                "a".repeat(64)
            )),
            "a captioned file is labelled by its caption: {gh}"
        );
        assert!(
            gh.contains(&format!(
                "- [notes.md](https://maidan.example.com/artifacts/{})",
                "b".repeat(64)
            )),
            "an uncaptioned file is labelled by its filename: {gh}"
        );

        let sl = delivery_body(tid, &slack(), &waiter, None);
        assert!(
            !sl.contains("/artifacts/") && !sl.contains("Attachments"),
            "attachment links are a GitHub-comment affordance only: {sl}"
        );
    }

    #[test]
    fn a_result_without_attachments_gains_no_attachment_lines() {
        let waiter = waiter(STATUS_REVIEWED, Some("## Findings"), Some("3 findings"));
        let gh = delivery_body(ThreadId::new(), &github(), &waiter, None);
        assert!(
            !gh.contains("/artifacts/") && !gh.contains("Attachments"),
            "no attachments, no extra lines: {gh}"
        );
    }

    #[test]
    fn an_attachment_caption_is_escaped_before_it_becomes_a_link_label() {
        let mut waiter = waiter(STATUS_REVIEWED, Some("## Findings"), Some("3 findings"));
        waiter.attachments = vec![attachment(
            &"c".repeat(64),
            "report.md",
            Some("Report [final] **v2** (draft) #1"),
        )];
        let gh = delivery_body(
            ThreadId::new(),
            &github(),
            &waiter,
            Some("https://maidan.example.com"),
        );
        assert!(
            gh.contains(&format!(
                "- [Report \\[final\\] \\*\\*v2\\*\\* \\(draft\\) \\#1](https://maidan.example.com/artifacts/{})",
                "c".repeat(64)
            )),
            "the caption renders literally, as data: {gh}"
        );
        assert!(
            !gh.contains("[final] **v2**"),
            "no raw Markdown from the caption survives: {gh}"
        );
    }

    #[test]
    fn without_a_public_origin_attachments_are_named_not_linked() {
        let mut waiter = waiter(STATUS_REVIEWED, Some("## Findings"), Some("3 findings"));
        waiter.attachments = vec![
            attachment(&"a".repeat(64), "trace.png", Some("The failing run")),
            attachment(&"b".repeat(64), "notes.md", None),
        ];
        let gh = delivery_body(ThreadId::new(), &github(), &waiter, None);
        assert!(
            gh.contains(
                "Attachments (open them from this thread in Maidan; they need Maidan auth):"
            ),
            "the header points at the thread, there are no links to follow: {gh}"
        );
        assert!(
            gh.contains("\n- The failing run") && gh.contains("\n- notes.md"),
            "each file is named, captioned first: {gh}"
        );
        assert!(
            !gh.contains("/artifacts/"),
            "a relative link would resolve against github.com and 404: {gh}"
        );
    }

    #[test]
    fn the_schema_constant_is_what_the_parser_routes_on() {
        // Guard against a silent rename: this module's contract is the 379.2
        // envelope, and a drift here would enqueue nothing for a real result.
        assert_eq!(WAITER_RESULT_SCHEMA, "maidan.waiter.result/1");
        assert_eq!(STATUS_REVIEWED, "reviewed");
    }

    fn sha40() -> String {
        "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911".into()
    }

    fn finding(file: &str, start: u32, end: u32, body: &str) -> WaiterFinding {
        WaiterFinding {
            file: file.into(),
            line_range: FindingLineRange::new(start, end).expect("valid range"),
            body: body.into(),
            severity: None,
        }
    }

    fn reviewed_with(head_sha: Option<String>, findings: Vec<WaiterFinding>) -> WaiterResult {
        WaiterResult {
            result_kind: "example.review.result/1".into(),
            status: STATUS_REVIEWED.into(),
            deliver_to: vec![],
            rendered: Some("## Findings".into()),
            summary: Some("1 finding".into()),
            view_url: None,
            pr: None,
            head_sha,
            findings,
            attachments: vec![],
        }
    }

    #[test]
    fn prepare_inline_review_maps_post_image_findings_onto_right_side() {
        let waiter = reviewed_with(
            Some(sha40()),
            vec![
                finding("auth.py", 2, 4, "bypass @octocat"),
                finding("auth.py", 7, 7, "one line"),
            ],
        );
        let review = prepare_inline_review(&waiter).expect("reviewed + sha + findings");
        assert_eq!(review.commit_id, sha40());
        assert!(!review.truncated);
        assert_eq!(review.comments.len(), 2);

        assert_eq!(review.comments[0].path, "auth.py");
        assert_eq!(review.comments[0].line, 4);
        assert_eq!(review.comments[0].start_line, Some(2));
        assert_eq!(review.comments[0].side, GithubDiffSide::Right);
        assert_eq!(review.comments[0].side.as_str(), "RIGHT");
        assert_eq!(review.comments[0].body, "bypass `@octocat`");
        assert!(
            !review.comments[0].body.contains("<!-- maidan:result:"),
            "inline comments must not carry the 379 marker"
        );

        assert_eq!(review.comments[1].line, 7);
        assert_eq!(
            review.comments[1].start_line, None,
            "a single-line finding omits start_line"
        );
    }

    #[test]
    fn prepare_inline_review_skips_without_head_sha_or_findings_or_reviewed() {
        let findings = vec![finding("auth.py", 1, 1, "x")];
        assert!(
            prepare_inline_review(&reviewed_with(None, findings.clone())).is_none(),
            "no commit_id ⇒ no review; the summary path still posts"
        );
        assert!(prepare_inline_review(&reviewed_with(Some(sha40()), vec![])).is_none());

        let mut failed = reviewed_with(Some(sha40()), findings);
        failed.status = "failed".into();
        assert!(
            prepare_inline_review(&failed).is_none(),
            "a non-reviewed result delivers a failure notice, never findings"
        );
    }

    #[test]
    fn prepare_inline_review_caps_at_githubs_comment_limit() {
        let findings = (1..=GITHUB_REVIEW_COMMENTS_MAX as u32 + 3)
            .map(|n| finding("a.rs", n, n, "body"))
            .collect();
        let review = prepare_inline_review(&reviewed_with(Some(sha40()), findings))
            .expect("over-cap still posts the first 100");
        assert!(review.truncated);
        assert_eq!(review.comments.len(), GITHUB_REVIEW_COMMENTS_MAX);
        assert_eq!(review.comments[0].line, 1);
        assert_eq!(
            review.comments[GITHUB_REVIEW_COMMENTS_MAX - 1].line,
            GITHUB_REVIEW_COMMENTS_MAX as u32
        );
    }

    #[test]
    fn prepare_inline_review_commit_id_is_only_the_envelope_sha() {
        // There is no helper that resolves a PR head. If one appears, this
        // test is the wrong place to use it — delete the helper.
        let waiter = reviewed_with(Some(sha40()), vec![finding("a.rs", 1, 1, "x")]);
        let review = prepare_inline_review(&waiter).unwrap();
        assert_eq!(review.commit_id, waiter.review_commit_id().unwrap());
        assert_eq!(review.commit_id, waiter.head_sha.as_deref().unwrap());
    }
}
