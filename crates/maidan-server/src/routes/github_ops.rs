//! The change flow's mark-ready action: flip a draft agent pull request to
//! ready for review. Callable only by the Soundcheck app, whose own token
//! cannot mark ready.

use axum::{extract::State, Extension, Json};
use maidan_auth::AuthContext;
use maidan_types::{AuditScope, EgressSurface, NewAuditEvent};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::ApiResult;
use crate::error::ApiError;
use crate::extract::ApiJson;
use crate::github::{GithubError, MarkReadyOutcome};
use crate::state::AppState;

/// The app allowed to ask for the flip: the change flow's companion, which
/// holds no GitHub write credential of its own (Decisions, 2026-10-06).
pub const SOUNDCHECK_APP_SLUG: &str = "soundcheck";

#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkReadyRequest {
    /// `owner/name`.
    pub repo: String,
    pub pull_number: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MarkReadyResponse {
    pub number: i64,
    pub marked_ready: bool,
    pub reason: Option<String>,
}

/// The caller's app slug, or a refusal. Only the Soundcheck installation may
/// ask for the flip: a member token, a session, or another app's token is
/// not Soundcheck, whatever capabilities it carries.
async fn soundcheck_caller(state: &AppState, auth: &AuthContext) -> ApiResult<()> {
    let refused =
        || ApiError::Forbidden("only the soundcheck app may mark pull requests ready".into());
    let installation_id = auth.app_installation_id.ok_or_else(refused)?;
    let installation = state
        .store
        .get_app_installation(installation_id)
        .await
        .map_err(|_| refused())?;
    // The installation belongs to the caller's workspace by construction, but
    // a cross-workspace installation id must not pass on a token's say-so.
    if installation.workspace_id != auth.workspace_id {
        return Err(refused());
    }
    let app = state
        .store
        .get_app(installation.app_id)
        .await
        .map_err(|_| refused())?;
    if app.slug != SOUNDCHECK_APP_SLUG {
        return Err(refused());
    }
    Ok(())
}

/// A GitHub failure as an HTTP status. A refusal is never an `Err` — the
/// client returns it as an outcome — so anything here is a real failure.
fn github_api_error(err: GithubError) -> ApiError {
    match err {
        GithubError::Api { status: 404, .. } => ApiError::NotFound,
        GithubError::Refused(reason) => ApiError::Forbidden(reason),
        other => ApiError::BadGateway(format!("github: {other}")),
    }
}

/// Best-effort audit of a mark-ready call. Never fails the request.
async fn audit_mark_ready(
    state: &AppState,
    auth: &AuthContext,
    repo: &str,
    pull_number: i64,
    head: Option<&str>,
    base: Option<&str>,
    outcome: &str,
    reason: Option<&str>,
) {
    crate::audit::record(
        state,
        NewAuditEvent {
            scope: AuditScope::Workspace(auth.workspace_id),
            actor_id: Some(auth.actor_id),
            action: "github.mark_ready".into(),
            target_kind: Some("github_pull".into()),
            target_id: None,
            metadata: serde_json::json!({
                "repo": repo,
                "pull_number": pull_number,
                "head": head,
                "base": base,
                "outcome": outcome,
                "reason": reason,
                "app_installation_id": auth.app_installation_id.map(|id| id.0),
            }),
        },
    )
    .await;
}

/// `POST /operator/github/mark-ready` — flip a draft pull request to ready
/// for review.
///
/// Soundcheck-only. The flip lands only on a `feature/agent-*` head into the
/// workspace's allowlisted base for that repo — never prod, never a merge,
/// never any other PR mutation. Every call is audited.
pub async fn mark_pull_ready(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<MarkReadyRequest>,
) -> ApiResult<Json<MarkReadyResponse>> {
    soundcheck_caller(&state, &auth).await?;
    let repo = body.repo.trim();
    if repo.is_empty() || body.pull_number <= 0 {
        return Err(ApiError::BadRequest(
            "repo and a positive pull_number are required".into(),
        ));
    }
    let sender = state
        .github_sender
        .as_ref()
        .ok_or_else(|| ApiError::Internal("github is not configured".into()))?;
    let git = sender
        .git()
        .ok_or_else(|| ApiError::Internal("github sender cannot mutate pull requests".into()))?;

    // Read before the allowlist: the selector needs the PR's actual base,
    // not one the caller claims.
    let brief = match git.pull_brief(repo, body.pull_number).await {
        Ok(brief) => brief,
        Err(err) => {
            let reason = format!("github: {err}");
            audit_mark_ready(
                &state,
                &auth,
                repo,
                body.pull_number,
                None,
                None,
                "failed",
                Some(&reason),
            )
            .await;
            return Err(github_api_error(err));
        }
    };
    if let Err(reason) = maidan_types::check_change_target(&brief.head, &brief.base) {
        audit_mark_ready(
            &state,
            &auth,
            repo,
            body.pull_number,
            Some(&brief.head),
            Some(&brief.base),
            "refused",
            Some(&reason),
        )
        .await;
        return Err(ApiError::Forbidden(reason));
    }
    // The same allowlist the change flow checks: `owner/name@base`, as it is
    // now — a blessing revoked after the draft opened stops the flip.
    let selector = maidan_types::change_allowlist_selector(repo, &brief.base);
    let allowed = state
        .store
        .is_egress_target_allowed(auth.workspace_id, EgressSurface::GithubBranch, &selector)
        .await?;
    if !allowed {
        let reason = format!("`{selector}` is not in the workspace egress allowlist");
        audit_mark_ready(
            &state,
            &auth,
            repo,
            body.pull_number,
            Some(&brief.head),
            Some(&brief.base),
            "refused",
            Some(&reason),
        )
        .await;
        return Err(ApiError::Forbidden(reason));
    }

    let response = match git.mark_pull_request_ready(repo, body.pull_number).await {
        Ok(MarkReadyOutcome::Marked { number, head, base }) => {
            crate::metrics::record_github_mark_ready("marked");
            audit_mark_ready(
                &state,
                &auth,
                repo,
                number,
                Some(&head),
                Some(&base),
                "marked",
                None,
            )
            .await;
            MarkReadyResponse {
                number,
                marked_ready: true,
                reason: None,
            }
        }
        Ok(MarkReadyOutcome::AlreadyReady { number }) => {
            crate::metrics::record_github_mark_ready("already_ready");
            audit_mark_ready(
                &state,
                &auth,
                repo,
                number,
                Some(&brief.head),
                Some(&brief.base),
                "already_ready",
                None,
            )
            .await;
            MarkReadyResponse {
                number,
                marked_ready: false,
                reason: Some("already ready".into()),
            }
        }
        Ok(MarkReadyOutcome::Refused(reason)) => {
            crate::metrics::record_github_mark_ready("refused");
            audit_mark_ready(
                &state,
                &auth,
                repo,
                body.pull_number,
                Some(&brief.head),
                Some(&brief.base),
                "refused",
                Some(&reason),
            )
            .await;
            return Err(ApiError::Forbidden(reason));
        }
        Err(err) => {
            let reason = format!("github: {err}");
            audit_mark_ready(
                &state,
                &auth,
                repo,
                body.pull_number,
                Some(&brief.head),
                Some(&brief.base),
                "failed",
                Some(&reason),
            )
            .await;
            return Err(github_api_error(err));
        }
    };
    Ok(Json(response))
}
