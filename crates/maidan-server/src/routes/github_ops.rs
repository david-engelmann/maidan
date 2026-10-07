//! The change flow's mark-ready action: flip a draft agent pull request to
//! ready for review. Callable only by the Soundcheck app, whose own token
//! cannot mark ready.

use axum::{extract::State, Extension, Json};
use maidan_auth::AuthContext;
use maidan_types::{AuditScope, EgressSurface, NewAuditEvent, WorkspaceId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::ApiResult;
use crate::error::ApiError;
use crate::extract::ApiJson;
use crate::github::{GithubError, GithubGit, MarkReadyOutcome};
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

/// Proof that the mark-ready guard path ran. `GithubGit::set_pull_ready`
/// takes one, and the field is private, so only this module can mint it —
/// the single construction site is `flip_pull_ready_guarded`, after the
/// shape, base-map and allowlist guards pass. This closes the bypass where
/// the bare trait method was directly callable, skipping every guard.
pub struct MarkReadyGuardPass {
    _private: (),
}

impl MarkReadyGuardPass {
    /// E2E harness only: the client-half contract tests call the bare
    /// `set_pull_ready` to pin its wire shape. It cannot be `cfg(test)`,
    /// because integration tests build the crate without that cfg, so the
    /// type system alone does not stop a caller. `mark_ready_e2e`'s
    /// `no_source_file_mints_a_guard_pass_for_tests` fails if any file under
    /// a crate's `src/` calls it; every production pass is minted in
    /// `flip_pull_ready_guarded`.
    #[doc(hidden)]
    pub fn for_tests() -> Self {
        Self { _private: () }
    }
}

/// What the single guard path can report. A refusal means nothing was
/// written; a store or GitHub failure means the flip did not happen, except
/// that a transport failure on the mutation itself cannot say which side of
/// the write it landed on — success is only reported when the mutation's
/// answer says `isDraft: false`.
#[derive(Debug)]
pub enum MarkReadyGuard {
    Refused(String),
    Store(maidan_store::StoreError),
    Github(GithubError),
}

/// The one guard path for the mark-ready flip (Decisions, 2026-10-06): a
/// single fresh pull brief, every guard evaluated against it — the PR-shape
/// rules, the per-repository base map, the workspace egress allowlist — and
/// only then the `markPullRequestReadyForReview` mutation, keyed on the
/// brief's node id. There is deliberately no second read: the allowlist
/// check used to run on an earlier brief than the write's, so a base retarget
/// in between slipped through. Every guard now sees the same brief the write
/// acts on.
///
/// Residual: the read and the mutation are still two GitHub calls with no
/// conditional write between them, so a retarget inside that window cannot be
/// refused. GitHub offers no precondition on the mutation; the window is one
/// round trip.
pub async fn flip_pull_ready_guarded(
    store: &dyn maidan_store::Store,
    workspace_id: WorkspaceId,
    git: &dyn GithubGit,
    repo: &str,
    pull_number: i64,
) -> Result<MarkReadyOutcome, MarkReadyGuard> {
    let brief = git
        .pull_brief(repo, pull_number)
        .await
        .map_err(MarkReadyGuard::Github)?;
    // Shape rules and the per-repository base map, on the fresh read.
    maidan_types::check_mark_ready_target(repo, &brief.head, &brief.base)
        .map_err(MarkReadyGuard::Refused)?;
    // A fork's branch can carry a `feature/agent-*` name too; only the change
    // flow's own branches, which live in the base repository, are flipped.
    if !brief.same_repo {
        return Err(MarkReadyGuard::Refused(
            "the pull request's head is not in this repository (a fork or a deleted head); \
             only the change flow's own branches are marked ready"
                .into(),
        ));
    }
    // The same allowlist the change flow checks: `owner/name@base`, as it is
    // now — a blessing revoked after the draft opened stops the flip.
    let selector = maidan_types::change_allowlist_selector(repo, &brief.base);
    let allowed = store
        .is_egress_target_allowed(workspace_id, EgressSurface::GithubBranch, &selector)
        .await
        .map_err(MarkReadyGuard::Store)?;
    if !allowed {
        return Err(MarkReadyGuard::Refused(format!(
            "`{selector}` is not in the workspace egress allowlist"
        )));
    }
    if !brief.draft {
        return Ok(MarkReadyOutcome::AlreadyReady {
            number: brief.number,
            head: brief.head,
            base: brief.base,
        });
    }
    git.set_pull_ready(
        repo,
        pull_number,
        &brief.node_id,
        MarkReadyGuardPass { _private: () },
    )
    .await
    .map_err(MarkReadyGuard::Github)?;
    Ok(MarkReadyOutcome::Marked {
        number: brief.number,
        head: brief.head,
        base: brief.base,
    })
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

/// A GitHub failure as an HTTP status.
fn github_api_error(err: GithubError) -> ApiError {
    match err {
        GithubError::Api { status: 404, .. } => ApiError::NotFound,
        GithubError::Refused(reason) => ApiError::Forbidden(reason),
        other => ApiError::BadGateway(format!("github: {other}")),
    }
}

/// Best-effort audit of a mark-ready call. Never fails the request: this is
/// the same fail-open `audit::record` every other egress path uses (M-A5).
#[allow(clippy::too_many_arguments)]
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
    crate::metrics::record_github_mark_ready(outcome);
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

/// `owner/name`, each side one or more of `[A-Za-z0-9_.-]`. Anything else is
/// a 400: GitHub would reject it, and a path-shaped value must never reach
/// the URL builder unvalidated.
fn valid_repo_shape(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let part_ok = |p: &str| {
        !p.is_empty()
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    };
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) => part_ok(owner) && part_ok(name),
        _ => false,
    }
}

/// `POST /operator/github/mark-ready` — flip a draft pull request to ready
/// for review.
///
/// Soundcheck-only. The flip lands only on a `feature/agent-*` head into the
/// workspace's allowlisted base for that repo — never prod, never a merge,
/// never any other PR mutation. Every call that reaches the handler is
/// audited, including refusals: this endpoint is the sole gate for a PR
/// mutation, so unlike other egress paths it audits refused calls too
/// (Decisions, 2026-10-06). Middleware denials never reach the handler; they
/// are counted, not stored.
pub async fn mark_pull_ready(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<MarkReadyRequest>,
) -> ApiResult<Json<MarkReadyResponse>> {
    let repo = body.repo.trim().to_string();
    let pull_number = body.pull_number;

    if let Err(err) = soundcheck_caller(&state, &auth).await {
        audit_mark_ready(
            &state,
            &auth,
            &repo,
            pull_number,
            None,
            None,
            "refused",
            Some("the caller is not the soundcheck app"),
        )
        .await;
        return Err(err);
    }
    if repo.is_empty() || pull_number <= 0 {
        audit_mark_ready(
            &state,
            &auth,
            &repo,
            pull_number,
            None,
            None,
            "refused",
            Some("repo and a positive pull_number are required"),
        )
        .await;
        return Err(ApiError::BadRequest(
            "repo and a positive pull_number are required".into(),
        ));
    }
    if !valid_repo_shape(&repo) {
        audit_mark_ready(
            &state,
            &auth,
            &repo,
            pull_number,
            None,
            None,
            "refused",
            Some("repo must be owner/name"),
        )
        .await;
        return Err(ApiError::BadRequest("repo must be owner/name".into()));
    }
    let Some(sender) = state.github_sender.as_ref() else {
        audit_mark_ready(
            &state,
            &auth,
            &repo,
            pull_number,
            None,
            None,
            "failed",
            Some("github is not configured"),
        )
        .await;
        return Err(ApiError::Internal("github is not configured".into()));
    };
    let Some(git) = sender.git() else {
        audit_mark_ready(
            &state,
            &auth,
            &repo,
            pull_number,
            None,
            None,
            "failed",
            Some("github sender cannot mutate pull requests"),
        )
        .await;
        return Err(ApiError::Internal(
            "github sender cannot mutate pull requests".into(),
        ));
    };

    let outcome = match flip_pull_ready_guarded(
        state.store.as_ref(),
        auth.workspace_id,
        git,
        &repo,
        pull_number,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(MarkReadyGuard::Refused(reason)) => {
            audit_mark_ready(
                &state,
                &auth,
                &repo,
                pull_number,
                None,
                None,
                "refused",
                Some(&reason),
            )
            .await;
            return Err(ApiError::Forbidden(reason));
        }
        Err(MarkReadyGuard::Store(err)) => {
            let reason = format!("allowlist check failed: {err}");
            audit_mark_ready(
                &state,
                &auth,
                &repo,
                pull_number,
                None,
                None,
                "failed",
                Some(&reason),
            )
            .await;
            return Err(ApiError::Internal(reason));
        }
        Err(MarkReadyGuard::Github(err)) => {
            let reason = format!("github: {err}");
            audit_mark_ready(
                &state,
                &auth,
                &repo,
                pull_number,
                None,
                None,
                "failed",
                Some(&reason),
            )
            .await;
            return Err(github_api_error(err));
        }
    };

    match outcome {
        MarkReadyOutcome::Marked { number, head, base } => {
            audit_mark_ready(
                &state,
                &auth,
                &repo,
                number,
                Some(&head),
                Some(&base),
                "marked",
                None,
            )
            .await;
            Ok(Json(MarkReadyResponse {
                number,
                marked_ready: true,
                reason: None,
            }))
        }
        MarkReadyOutcome::AlreadyReady { number, head, base } => {
            audit_mark_ready(
                &state,
                &auth,
                &repo,
                number,
                Some(&head),
                Some(&base),
                "already_ready",
                None,
            )
            .await;
            Ok(Json(MarkReadyResponse {
                number,
                marked_ready: false,
                reason: Some("already ready".into()),
            }))
        }
    }
}
