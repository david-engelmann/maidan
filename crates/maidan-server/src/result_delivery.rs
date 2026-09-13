//! Result delivery trigger (Cluster 379.3).
//!
//! A `ThreadResultSet` is a "go fetch" pointer (Cluster 235): the envelope lives
//! on the thread, not on the event. This module is the arm of
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
//! The every-replica router is safe because `arm_result_delivery` is
//! the contended write: exactly one replica wins the right to enqueue. The
//! egress outbox is *transport* (retry/backoff); this module only arms intent
//! and, for a blessed target, puts an [`EgressKind::Result`] row on that queue.
//! The worker (Cluster 379.4) updates in place via the stored `external_ref`,
//! recovering a GitHub comment through the hidden body marker if the handle
//! is lost.
//!
//! Cluster 380.2 / 380.3: after a successful GitHub summary comment, the
//! worker POSTs a `COMMENT` review whose `commit_id` is envelope `head_sha`
//! and whose inline comments are the usable findings (RIGHT, post-image
//! `line_range`). A missing sha, empty findings, a non-`reviewed` status, a
//! vanished envelope, or a GitHub 404/422 skips the review; a 5xx is left
//! for operator replay. The 379 summary path is unchanged.

use chrono::{DateTime, Utc};
use maidan_types::{
    parse_waiter_result, status, DeliverTarget, EgressKind, EgressTarget, GithubReviewComment,
    NewEgressOutbox, ResultDelivery, ThreadId, WaiterResult, WorkspaceId,
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
/// ceiling) with a hidden `<!-- maidan:result:<thread_id> -->` marker at byte
/// 0 — the Cluster 379.4 recovery path if the stored `external_ref` is lost.
/// Slack gets `summary` plus a compact digest — **never** `rendered`, which
/// is GFM and would arrive visibly broken. A non-`reviewed` status replaces
/// both with [`failure_notice`].
pub fn delivery_body(thread_id: ThreadId, target: &EgressTarget, waiter: &WaiterResult) -> String {
    let inner = if waiter.is_reviewed() {
        reviewed_body(target, waiter)
    } else {
        // Still marked on GitHub so a later reviewed result updates this
        // comment rather than stacking a second one.
        failure_notice(&waiter.status)
    };
    match target {
        EgressTarget::Github { .. } => {
            let backlink = waiter
                .is_reviewed()
                .then_some(waiter.view_in_pi.as_deref())
                .flatten();
            // Failure notices are already the complete inner body; reviewed
            // inner is the (PR-annotated) rendered. The helper defuses and
            // truncates either way, reserving the marker in the budget.
            github_result_comment_body(thread_id, &inner, backlink)
        }
        EgressTarget::Slack { .. } => inner,
    }
}

fn reviewed_body(target: &EgressTarget, waiter: &WaiterResult) -> String {
    let backlink = waiter.view_in_pi.as_deref();
    match target {
        EgressTarget::Github { .. } => {
            let mut rendered = waiter.rendered.clone().unwrap_or_default();
            if let Some(pr) = &waiter.pr {
                if !rendered.is_empty() {
                    rendered.push_str("\n\n");
                }
                rendered.push_str("PR: ");
                rendered.push_str(pr);
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
    for target in &waiter.deliver_to {
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

/// Cluster 383.2: feed a delivered (or thread-only) `pi.review.result/1`
/// with any `critical` finding into the Cluster-375 close-gate. Uses the
/// result's `produced_by` as the reviewer — they must have declared the
/// `review` skill. Empty `deliver_to` still arms: the room blocks the land
/// even when nothing is posted externally. Idempotent (upsert + `k` only
/// when unset) so every replica of the router can call it.
pub async fn arm_critical_review(state: &AppState, thread_id: ThreadId) -> Result<(), String> {
    let stored = match state.store.get_thread_result(thread_id).await {
        Ok(Some(stored)) => stored,
        Ok(None) => return Ok(()),
        Err(err) => return Err(err.to_string()),
    };
    state
        .store
        .apply_critical_review_decision(thread_id, stored.produced_by, &stored.result)
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
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
            let allowed = state
                .store
                .is_egress_target_allowed(
                    workspace_id,
                    egress.surface(),
                    &egress.allowlist_selector(),
                )
                .await
                .map_err(|e| e.to_string())?;
            if allowed {
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
    state
        .store
        .mark_result_delivery_skipped(row.id, reason)
        .await
        .map_err(|e| e.to_string())?;
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
    let body = delivery_body(thread_id, target, waiter);
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
    Some(delivery_body(thread_id, target, &waiter))
}

/// The live waiter envelope, if the thread still has one Maidan recognizes.
pub async fn current_waiter(state: &AppState, thread_id: ThreadId) -> Option<WaiterResult> {
    let stored = state.store.get_thread_result(thread_id).await.ok()??;
    parse_waiter_result(&stored.result)
}

/// Coordinates for one `POST /repos/{repo}/pulls/{n}/reviews` (Cluster 380.2).
/// `None` means skip the inline review: the 379 summary comment still posts.
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
    actor_id: Option<maidan_types::MemberId>,
    row: &ResultDelivery,
) {
    crate::audit::record(
        state,
        maidan_types::NewAuditEvent {
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
    use super::*;
    use maidan_types::{
        FindingLineRange, GithubDiffSide, WaiterFinding, STATUS_REVIEWED, WAITER_RESULT_SCHEMA,
    };

    fn waiter(status: &str, rendered: Option<&str>, summary: Option<&str>) -> WaiterResult {
        WaiterResult {
            result_kind: "pi.review.result/1".into(),
            status: status.into(),
            deliver_to: vec![],
            rendered: rendered.map(str::to_string),
            summary: summary.map(str::to_string),
            view_in_pi: Some("https://pi.test/r/1".into()),
            pr: Some("acme/widgets#7".into()),
            head_sha: None,
            findings: vec![],
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
            let body = delivery_body(tid, &target, &waiter);
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
        let gh = delivery_body(tid, &github(), &waiter);
        assert!(
            crate::egress_body::comment_carries_result_marker(&gh, tid),
            "a failure notice on GitHub is still marked so a later review updates it: {gh}"
        );
        let sl = delivery_body(tid, &slack(), &waiter);
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
        let gh = delivery_body(tid, &github(), &waiter);
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
        assert!(gh.contains("https://pi.test/r/1"));

        let sl = delivery_body(tid, &slack(), &waiter);
        assert!(sl.contains("3 findings"), "slack delivers the summary");
        assert!(
            !sl.contains("Findings") && !sl.contains("@octocat"),
            "slack must never receive rendered GFM: {sl}"
        );
        assert!(sl.contains("PR: acme/widgets#7"));
        assert!(sl.contains("https://pi.test/r/1"));
        assert!(
            !sl.contains("<!-- maidan:result:"),
            "slack has no HTML-comment recovery marker"
        );
    }

    #[test]
    fn the_schema_constant_is_what_the_parser_routes_on() {
        // Guard against a silent rename: this module's contract is the 379.2
        // envelope, and a drift here would enqueue nothing for a real result.
        assert_eq!(WAITER_RESULT_SCHEMA, "pi.waiter.result/1");
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
            result_kind: "pi.review.result/1".into(),
            status: STATUS_REVIEWED.into(),
            deliver_to: vec![],
            rendered: Some("## Findings".into()),
            summary: Some("1 finding".into()),
            view_in_pi: None,
            pr: None,
            head_sha,
            findings,
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
