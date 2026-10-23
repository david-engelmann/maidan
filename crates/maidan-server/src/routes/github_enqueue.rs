//! The change flow's merge-queue actions: label a ready agent pull request
//! so the repository's merge queue takes it, read whether it is labelled, and
//! take the label off again. Callable only by the mark-ready app, after
//! mark-ready, once the producer's own checks passed.
//!
//! Maidan only applies or removes the label. What the label does is the
//! repository's merge-queue setting, which its owner controls, and a
//! repository with no label in `MAIDAN_MERGE_QUEUE_LABELS` is never enqueued:
//! a person merges its changes.

use axum::{extract::State, Extension, Json};
use maidan_auth::AuthContext;
use maidan_types::{AuditScope, EgressSurface, NewAuditEvent, WorkspaceId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::github_ops::{
    github_api_error, require_mark_ready_app, valid_repo_shape, MarkReadyGuard,
};
use super::ApiResult;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiQuery};
use crate::github::GithubGit;
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EnqueueRequest {
    /// `owner/name`.
    pub repo: String,
    pub pull_number: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EnqueueResponse {
    pub number: i64,
    /// The repository's merge-queue label.
    pub label: String,
    /// Whether the pull request carries the label now.
    pub applied: bool,
    pub reason: Option<String>,
}

/// Proof that an enqueue or dequeue guard path ran; only this module mints
/// one, and [`GithubGit::add_pull_label`] and [`GithubGit::remove_pull_label`]
/// take one, as for mark-ready's [`super::MarkReadyGuardPass`].
pub struct EnqueueGuardPass {
    _private: (),
}

/// What the enqueue guard path did.
#[derive(Debug, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Labelled { number: i64, label: String },
    AlreadyLabelled { number: i64, label: String },
}

/// The label `repo` enqueues with, or the refusal for a repository a person
/// merges.
fn label_for<'a>(
    labels: &'a maidan_types::MergeQueueLabels,
    repo: &str,
) -> Result<&'a str, MarkReadyGuard> {
    labels.label_for(repo).ok_or_else(|| {
        MarkReadyGuard::Refused(format!(
            "`{repo}` has no merge-queue label in {}; a person merges its changes",
            maidan_types::MERGE_QUEUE_LABELS_ENV
        ))
    })
}

/// The guards enqueue and dequeue share, on one fresh read: mark-ready's
/// shape rules and base pins (a `feature/agent-*` head into its pinned base,
/// never prod), a head in the base repository, and the `owner/name@base`
/// blessing in the workspace's `github_branch` allowlist.
async fn shared_guards(
    store: &dyn maidan_store::Store,
    workspace_id: WorkspaceId,
    bases: &maidan_types::MarkReadyBases,
    repo: &str,
    brief: &crate::github::GithubPullBrief,
) -> Result<(), MarkReadyGuard> {
    maidan_types::check_mark_ready_target(bases, repo, &brief.head, &brief.base)
        .map_err(MarkReadyGuard::Refused)?;
    if !brief.same_repo {
        return Err(MarkReadyGuard::Refused(
            "the pull request's head is not in this repository (a fork or a deleted head)".into(),
        ));
    }
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
    Ok(())
}

/// The one guard path for enqueue. One fresh pull read, and every guard on
/// it: a label is configured for the repository, the shared guards, the pull
/// request is open and ready (not a draft), and its head commit is the
/// commit the change flow last delivered to it, so nothing anyone pushed
/// afterwards rides into the queue unchecked. Then the label, unless it is
/// there already.
pub async fn enqueue_pull_guarded(
    store: &dyn maidan_store::Store,
    workspace_id: WorkspaceId,
    git: &dyn GithubGit,
    bases: &maidan_types::MarkReadyBases,
    labels: &maidan_types::MergeQueueLabels,
    repo: &str,
    pull_number: i64,
) -> Result<EnqueueOutcome, MarkReadyGuard> {
    let label = label_for(labels, repo)?;
    let brief = git
        .pull_brief(repo, pull_number)
        .await
        .map_err(MarkReadyGuard::Github)?;
    shared_guards(store, workspace_id, bases, repo, &brief).await?;
    if !brief.open {
        return Err(MarkReadyGuard::Refused(format!(
            "pull request #{} is closed",
            brief.number
        )));
    }
    if brief.draft {
        return Err(MarkReadyGuard::Refused(format!(
            "pull request #{} is a draft; mark it ready before enqueueing it",
            brief.number
        )));
    }
    match store
        .delivered_change_head(workspace_id, repo, pull_number)
        .await
        .map_err(MarkReadyGuard::Store)?
    {
        None => {
            return Err(MarkReadyGuard::Refused(format!(
                "the change flow delivered nothing to #{pull_number} in this workspace"
            )));
        }
        Some(sha) if !sha.eq_ignore_ascii_case(&brief.head_sha) => {
            return Err(MarkReadyGuard::Refused(format!(
                "the head of #{pull_number} is {}, not {sha}, the commit the change flow delivered",
                brief.head_sha
            )));
        }
        Some(_) => {}
    }
    if brief.labels.iter().any(|name| name == label) {
        return Ok(EnqueueOutcome::AlreadyLabelled {
            number: brief.number,
            label: label.to_string(),
        });
    }
    git.add_pull_label(repo, pull_number, label, EnqueueGuardPass { _private: () })
        .await
        .map_err(MarkReadyGuard::Github)?;
    Ok(EnqueueOutcome::Labelled {
        number: brief.number,
        label: label.to_string(),
    })
}

/// The guard path for dequeue: the shared guards on one fresh read, but not
/// the head comparison (the merge queue rebases a queued pull request
/// itself), and a closed pull request may still lose its label. Then the
/// label comes off, if it is on. Returns the number, the label, and whether
/// a label was removed.
pub async fn dequeue_pull_guarded(
    store: &dyn maidan_store::Store,
    workspace_id: WorkspaceId,
    git: &dyn GithubGit,
    bases: &maidan_types::MarkReadyBases,
    labels: &maidan_types::MergeQueueLabels,
    repo: &str,
    pull_number: i64,
) -> Result<(i64, String, bool), MarkReadyGuard> {
    let label = label_for(labels, repo)?;
    let brief = git
        .pull_brief(repo, pull_number)
        .await
        .map_err(MarkReadyGuard::Github)?;
    shared_guards(store, workspace_id, bases, repo, &brief).await?;
    if !brief.labels.iter().any(|name| name == label) {
        return Ok((brief.number, label.to_string(), false));
    }
    let removed = git
        .remove_pull_label(repo, pull_number, label, EnqueueGuardPass { _private: () })
        .await
        .map_err(MarkReadyGuard::Github)?;
    Ok((brief.number, label.to_string(), removed))
}

async fn audit_enqueue(
    state: &AppState,
    auth: &AuthContext,
    repo: &str,
    pull_number: i64,
    outcome: &str,
    reason: Option<&str>,
) {
    crate::audit::record(
        state,
        NewAuditEvent {
            scope: AuditScope::Workspace(auth.workspace_id),
            actor_id: Some(auth.actor_id),
            action: "github.enqueue".into(),
            target_kind: Some("github_pull".into()),
            target_id: None,
            metadata: serde_json::json!({
                "repo": repo,
                "pull_number": pull_number,
                "outcome": outcome,
                "reason": reason,
                "app_installation_id": auth.app_installation_id.map(|id| id.0),
            }),
        },
    )
    .await;
}

fn checked_request(repo: &str, pull_number: i64) -> ApiResult<String> {
    let repo = repo.trim().to_string();
    if pull_number <= 0 || !valid_repo_shape(&repo) {
        return Err(ApiError::BadRequest(
            "repo (owner/name) and a positive pull_number are required".into(),
        ));
    }
    Ok(repo)
}

fn writer(state: &AppState) -> ApiResult<&dyn GithubGit> {
    state
        .github_sender
        .as_ref()
        .and_then(|sender| sender.git())
        .ok_or_else(|| ApiError::Internal("github is not configured to write".into()))
}

/// `POST /operator/github/enqueue`: label a ready agent pull request for its
/// repository's merge queue. Mark-ready app only; every call that reaches the
/// handler is audited, refusals included, as for mark-ready.
pub async fn enqueue_pull(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<EnqueueRequest>,
) -> ApiResult<Json<EnqueueResponse>> {
    if let Err(err) = require_mark_ready_app(&state, &auth).await {
        audit_enqueue(
            &state,
            &auth,
            &body.repo,
            body.pull_number,
            "refused",
            Some("the caller is not the operator-designated app"),
        )
        .await;
        return Err(err);
    }
    let repo = checked_request(&body.repo, body.pull_number)?;
    let pull_number = body.pull_number;
    let git = writer(&state)?;
    match enqueue_pull_guarded(
        state.store.as_ref(),
        auth.workspace_id,
        git,
        &state.mark_ready_bases,
        &state.merge_queue_labels,
        &repo,
        pull_number,
    )
    .await
    {
        Ok(EnqueueOutcome::Labelled { number, label }) => {
            audit_enqueue(&state, &auth, &repo, number, "labelled", None).await;
            Ok(Json(EnqueueResponse {
                number,
                label,
                applied: true,
                reason: None,
            }))
        }
        Ok(EnqueueOutcome::AlreadyLabelled { number, label }) => {
            audit_enqueue(&state, &auth, &repo, number, "already_labelled", None).await;
            Ok(Json(EnqueueResponse {
                number,
                label,
                applied: true,
                reason: Some("already labelled".into()),
            }))
        }
        Err(MarkReadyGuard::Refused(reason)) => {
            audit_enqueue(&state, &auth, &repo, pull_number, "refused", Some(&reason)).await;
            Err(ApiError::Forbidden(reason))
        }
        Err(MarkReadyGuard::Store(err)) => {
            let reason = format!("store: {err}");
            audit_enqueue(&state, &auth, &repo, pull_number, "failed", Some(&reason)).await;
            Err(ApiError::Internal(reason))
        }
        Err(MarkReadyGuard::Github(err)) => {
            let reason = format!("github: {err}");
            audit_enqueue(&state, &auth, &repo, pull_number, "failed", Some(&reason)).await;
            Err(github_api_error(err))
        }
    }
}

/// `GET /operator/github/enqueue?repo=&pull_number=`: whether the pull request
/// carries its repository's merge-queue label (`false` for one never
/// labelled). Mark-ready app only.
pub async fn enqueue_status(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiQuery(query): ApiQuery<EnqueueRequest>,
) -> ApiResult<Json<EnqueueResponse>> {
    require_mark_ready_app(&state, &auth).await?;
    let repo = checked_request(&query.repo, query.pull_number)?;
    let label = state
        .merge_queue_labels
        .label_for(&repo)
        .ok_or_else(|| {
            ApiError::Forbidden(format!(
                "`{repo}` has no merge-queue label in {}",
                maidan_types::MERGE_QUEUE_LABELS_ENV
            ))
        })?
        .to_string();
    let brief = writer(&state)?
        .pull_brief(&repo, query.pull_number)
        .await
        .map_err(github_api_error)?;
    let applied = brief.labels.iter().any(|name| name == &label);
    Ok(Json(EnqueueResponse {
        number: brief.number,
        label,
        applied,
        reason: None,
    }))
}

/// `DELETE /operator/github/enqueue?repo=&pull_number=`: take a pull request
/// out of its repository's merge queue, so a later commit can be pushed to
/// it. Mark-ready app only, audited like enqueue.
pub async fn dequeue_pull(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiQuery(query): ApiQuery<EnqueueRequest>,
) -> ApiResult<Json<EnqueueResponse>> {
    if let Err(err) = require_mark_ready_app(&state, &auth).await {
        audit_enqueue(
            &state,
            &auth,
            &query.repo,
            query.pull_number,
            "dequeue_refused",
            Some("the caller is not the operator-designated app"),
        )
        .await;
        return Err(err);
    }
    let repo = checked_request(&query.repo, query.pull_number)?;
    let pull_number = query.pull_number;
    let git = writer(&state)?;
    match dequeue_pull_guarded(
        state.store.as_ref(),
        auth.workspace_id,
        git,
        &state.mark_ready_bases,
        &state.merge_queue_labels,
        &repo,
        pull_number,
    )
    .await
    {
        Ok((number, label, removed)) => {
            let outcome = if removed { "dequeued" } else { "not_queued" };
            audit_enqueue(&state, &auth, &repo, number, outcome, None).await;
            Ok(Json(EnqueueResponse {
                number,
                label,
                applied: false,
                reason: (!removed).then(|| "was not labelled".to_string()),
            }))
        }
        Err(MarkReadyGuard::Refused(reason)) => {
            audit_enqueue(
                &state,
                &auth,
                &repo,
                pull_number,
                "dequeue_refused",
                Some(&reason),
            )
            .await;
            Err(ApiError::Forbidden(reason))
        }
        Err(MarkReadyGuard::Store(err)) => Err(ApiError::Internal(format!("store: {err}"))),
        Err(MarkReadyGuard::Github(err)) => Err(github_api_error(err)),
    }
}
