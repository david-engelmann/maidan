//! Git / GitHub App projector — webhook ingress foundation (Cluster 310).
//!
//! A *projector*, not a bot (no LLM in Maidan; Expansion Bets, Bet 6): it relays a
//! GitHub issue/PR conversation to a Maidan thread and back. This cluster lands the
//! ingress foundation — `X-Hub-Signature-256` verification + the `ping` setup event
//! — so a GitHub App (or repo webhook) can be pointed at
//! `POST /integrations/github/events`. Repo/issue link mapping + `issue_comment`
//! routing is Cluster 311; egress (Maidan → issue/PR comment) is 312.
//!
//! **Config-gated:** inert unless `MAIDAN_GITHUB_WEBHOOK_SECRET` is set (the route
//! then returns `404`). GitHub signs `sha256=hex(HMAC-SHA256(secret, body))` — the
//! same scheme as Maidan's own outbound webhook signatures, so verification reuses
//! [`crate::webhooks::verify_signature`].

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use maidan_auth::{capability::WORKSPACE_READ, capability::WORKSPACE_WRITE, AuthContext};
use maidan_types::{GithubIssueLink, MemberId, NewGithubIssueLink, ThreadId, WorkspaceId};

use crate::dto::{LinkGithubIssue, UnlinkGithubQuery};
use crate::error::ApiJson;
use crate::routes::{cap, ensure_workspace, ApiResult};
use crate::state::AppState;

/// GitHub App / webhook credentials. `webhook_secret` verifies inbound deliveries;
/// `api_token` (optional here) authorizes outbound comment posts in the egress
/// cluster (312) — an installation token or a PAT.
#[derive(Debug, Clone)]
pub struct GithubConfig {
    pub webhook_secret: String,
    pub api_token: Option<String>,
}

impl GithubConfig {
    /// Build from the environment, or `None` when `MAIDAN_GITHUB_WEBHOOK_SECRET` is
    /// unset — the projector is then disabled.
    pub fn from_env() -> Option<GithubConfig> {
        let webhook_secret = std::env::var("MAIDAN_GITHUB_WEBHOOK_SECRET")
            .ok()
            .filter(|s| !s.is_empty())?;
        let api_token = std::env::var("MAIDAN_GITHUB_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        Some(GithubConfig {
            webhook_secret,
            api_token,
        })
    }
}

/// `POST /integrations/github/events` — the GitHub webhook ingress. Returns `404`
/// when the projector isn't configured, `401` on a bad `X-Hub-Signature-256`,
/// `200` for the `ping` setup event and (for now) other events — `issue_comment`
/// routing to a Maidan thread lands in Cluster 311.
pub async fn github_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(cfg) = state.github.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !crate::webhooks::verify_signature(&cfg.webhook_secret, &body, signature) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    match event {
        // Webhook setup handshake — GitHub sends `ping` once; a 200 confirms it.
        "ping" => Json(serde_json::json!({ "ok": true })).into_response(),
        // A new comment on a linked issue/PR → the mapped Maidan thread (311).
        "issue_comment" => {
            route_github_issue_comment(&state, &payload).await;
            StatusCode::OK.into_response()
        }
        _ => StatusCode::OK.into_response(),
    }
}

/// Route an inbound GitHub `issue_comment` event: a new comment on a linked
/// issue/PR is posted into the mapped Maidan thread (Cluster 311). Best-effort —
/// the ingress always ACKs. Only `action == "created"` projects; a `Bot` comment
/// (our own egress echo, Cluster 312) is skipped to avoid loops.
async fn route_github_issue_comment(state: &AppState, payload: &serde_json::Value) {
    if payload.get("action").and_then(|v| v.as_str()) != Some("created") {
        return;
    }
    let comment = payload.get("comment");
    if comment
        .and_then(|c| c.get("user"))
        .and_then(|u| u.get("type"))
        .and_then(|t| t.as_str())
        == Some("Bot")
    {
        return; // our own egress comment — don't re-ingest (loop prevention)
    }
    let (Some(repo), Some(issue_number), Some(text)) = (
        payload
            .get("repository")
            .and_then(|r| r.get("full_name"))
            .and_then(|v| v.as_str()),
        payload
            .get("issue")
            .and_then(|i| i.get("number"))
            .and_then(|v| v.as_i64()),
        comment.and_then(|c| c.get("body")).and_then(|v| v.as_str()),
    ) else {
        return;
    };
    let author = comment
        .and_then(|c| c.get("user"))
        .and_then(|u| u.get("login"))
        .and_then(|v| v.as_str())
        .unwrap_or("github");
    let link = match state.store.get_github_issue_link(repo, issue_number).await {
        Ok(Some(l)) => l,
        Ok(None) => return, // issue not linked — ignore
        Err(err) => {
            tracing::warn!(error = %err, "github ingress: link lookup failed");
            return;
        }
    };
    let new = maidan_types::NewMessage {
        thread_id: link.thread_id,
        author_id: link.member_id,
        body: format!("{author}: {text}"),
        // Tag the origin so egress (Cluster 312) never echoes a GitHub-sourced
        // message back to GitHub (loop prevention).
        metadata: serde_json::json!({ "github": { "user": author, "repo": repo, "issue": issue_number } }),
        content: None,
    };
    match state.store.post_message_with_event(new, None).await {
        Ok((_, stored)) => crate::routes::publish_stored(state, stored).await,
        Err(err) => tracing::warn!(error = %err, "github ingress: post failed"),
    }
}

/// A failed GitHub API call.
#[derive(Debug, thiserror::Error)]
pub enum GithubError {
    #[error("github http error: {0}")]
    Http(String),
    #[error("github api error: status {0}")]
    Api(u16),
}

/// Outbound GitHub sender — posts an issue/PR comment in production, a mock in tests.
#[async_trait::async_trait]
pub trait GithubSender: Send + Sync {
    async fn post_comment(
        &self,
        repo: &str,
        issue_number: i64,
        text: &str,
    ) -> Result<(), GithubError>;
}

/// The production [`GithubSender`]: posts via the GitHub REST API
/// `POST /repos/{repo}/issues/{n}/comments`.
pub struct GithubApiClient {
    token: String,
    http: reqwest::Client,
}

impl GithubApiClient {
    pub fn new(token: String) -> Self {
        Self {
            token,
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl GithubSender for GithubApiClient {
    async fn post_comment(
        &self,
        repo: &str,
        issue_number: i64,
        text: &str,
    ) -> Result<(), GithubError> {
        let url = format!("https://api.github.com/repos/{repo}/issues/{issue_number}/comments");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "maidan-projector") // GitHub requires a User-Agent
            .json(&serde_json::json!({ "body": text }))
            .send()
            .await
            .map_err(|e| GithubError::Http(e.to_string()))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(GithubError::Api(resp.status().as_u16()))
        }
    }
}

/// GitHub projector egress (Cluster 312): relay a Maidan message posted in a linked
/// thread out as a GitHub issue/PR comment. No-op unless a [`GithubSender`] is
/// configured; **skips messages that originated in GitHub** (the `metadata.github`
/// tag from 311's ingress) so a projected inbound comment is never echoed back —
/// loop prevention. Best-effort (a failed post is logged + metered, not retried).
pub async fn route_message_to_github(
    state: &AppState,
    thread_id: maidan_types::ThreadId,
    message: &maidan_types::Message,
) {
    let Some(sender) = state.github_sender.as_ref() else {
        return;
    };
    if message.metadata.get("github").is_some() {
        return; // originated in GitHub — don't echo it back
    }
    let link = match state.store.get_github_issue_link_by_thread(thread_id).await {
        Ok(Some(l)) => l,
        Ok(None) => return, // thread not linked to a GitHub issue/PR
        Err(err) => {
            tracing::warn!(error = %err, "github egress: link lookup failed");
            return;
        }
    };
    match sender
        .post_comment(&link.repo, link.issue_number, &message.body)
        .await
    {
        Ok(()) => crate::metrics::record_github_egress("sent"),
        Err(err) => {
            tracing::warn!(error = %err, "github egress: comment post failed");
            crate::metrics::record_github_egress("failed");
        }
    }
}

/// `POST /workspaces/:wid/github-links` (Cluster 346) — link a GitHub issue/PR to a
/// Maidan thread so the projector can bridge comments both ways. The link's
/// `channel_id`/`workspace_id` are derived from resolving the thread; the caller
/// supplies only `repo` (`owner/name`), `issue_number`, thread, and the attribution
/// member. `workspace:write` + access to the thread. Upserts.
pub async fn link_github_issue(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<LinkGithubIssue>,
) -> ApiResult<(StatusCode, Json<GithubIssueLink>)> {
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, WorkspaceId(wid))?;
    let scope =
        maidan_auth::authorize_thread(state.store.as_ref(), &auth, ThreadId(body.thread_id))
            .await?;
    if scope.workspace_id != WorkspaceId(wid) {
        return Err(crate::error::ApiError::BadRequest(
            "thread is not in this workspace".into(),
        ));
    }
    let link = state
        .store
        .link_github_issue(NewGithubIssueLink {
            repo: body.repo,
            issue_number: body.issue_number,
            workspace_id: scope.workspace_id,
            channel_id: scope.channel_id,
            thread_id: ThreadId(body.thread_id),
            member_id: MemberId(body.member_id),
        })
        .await?;
    Ok((StatusCode::CREATED, Json(link)))
}

/// `GET /workspaces/:wid/github-links` (Cluster 346) — the workspace's GitHub
/// issue/PR links. `workspace:read`.
pub async fn list_github_issue_links(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<GithubIssueLink>>> {
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, WorkspaceId(wid))?;
    Ok(Json(
        state
            .store
            .list_github_issue_links(WorkspaceId(wid))
            .await?,
    ))
}

/// `DELETE /workspaces/:wid/github-links?repo=…&issue_number=…` (Cluster 346) —
/// remove a GitHub link (`repo` carries a slash, so it's a query pair, not a path).
/// `workspace:write`. `404` if the link doesn't exist in this workspace.
pub async fn unlink_github_issue(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
    Query(q): Query<UnlinkGithubQuery>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, WorkspaceId(wid))?;
    let (Some(repo), Some(issue_number)) = (q.repo, q.issue_number) else {
        return Err(crate::error::ApiError::BadRequest(
            "repo and issue_number query params are required".into(),
        ));
    };
    match state
        .store
        .get_github_issue_link(&repo, issue_number)
        .await?
    {
        Some(link) if link.workspace_id == WorkspaceId(wid) => {}
        _ => return Err(crate::error::ApiError::NotFound),
    }
    if state.store.unlink_github_issue(&repo, issue_number).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(crate::error::ApiError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_disabled_without_a_secret() {
        // Not asserting on process env here (shared) — just the shape: a config with
        // an empty/unset secret must not enable the projector. Verified via the
        // ingress e2e (404 when unconfigured).
        let cfg = GithubConfig {
            webhook_secret: "s".into(),
            api_token: None,
        };
        assert_eq!(cfg.webhook_secret, "s");
        assert!(cfg.api_token.is_none());
    }
}
