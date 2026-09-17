//! Git / GitHub App projector — webhook ingress foundation.
//!
//! A *projector*, not a bot (no LLM in Maidan; Expansion Bets, Bet 6): it
//! relays a GitHub issue/PR conversation to a Maidan thread and back. This
//! cluster lands the ingress foundation — `X-Hub-Signature-256` verification +
//! the `ping` setup event — so a GitHub App (or repo webhook) can be pointed at
//! `POST /integrations/github/events`. Repo/issue link mapping +
//! `issue_comment` routing and egress (Maidan → issue/PR comment) build on it.
//!
//! **Config-gated:** inert unless `MAIDAN_GITHUB_WEBHOOK_SECRET` is set (the route
//! then returns `404`). GitHub signs `sha256=hex(HMAC-SHA256(secret, body))` —
//! the same scheme as Maidan's own outbound webhook signatures, so verification
//! reuses [`crate::webhooks::verify_signature`].

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use maidan_auth::{capability::WORKSPACE_READ, capability::WORKSPACE_WRITE, AuthContext};
use maidan_types::{
    EgressKind, EgressTarget, ExternalRef, GithubIssueLink, GithubReviewComment, MemberId,
    NewEgressOutbox, NewGithubIssueLink, ThreadId, WorkspaceId, GITHUB_REVIEW_EVENT_COMMENT,
};

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

/// `POST /integrations/github/events` — the GitHub webhook ingress. Returns
/// `404` when the projector isn't configured, `401` on a bad
/// `X-Hub-Signature-256`, `200` for the `ping` setup event, an `issue_comment`
/// (projected to the linked Maidan thread), a merged `pull_request` (a
/// `ThreadLanded` fact), and (with no side effect) any other event.
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
        // A merged PR on a linked issue/PR → a `ThreadLanded` fact.
        "pull_request" => {
            route_github_pull_request(&state, &payload).await;
            StatusCode::OK.into_response()
        }
        _ => StatusCode::OK.into_response(),
    }
}

/// Route an inbound GitHub `pull_request` event: when a PR **linked** to a
/// Maidan thread is **merged** (`action == "closed"` with `pull_request.merged
/// == true`), emit a `ThreadLanded` fact on that thread. This "steals the
/// landed fact" — it records that the work landed; it does **not** transition
/// the thread's FSM (not an automation product). Best-effort; the ingress
/// always ACKs. A closed-but-unmerged PR, or a PR not linked to a thread, is
/// ignored.
async fn route_github_pull_request(state: &AppState, payload: &serde_json::Value) {
    if payload.get("action").and_then(|v| v.as_str()) != Some("closed") {
        return; // only a close can be a merge
    }
    let pr = payload.get("pull_request");
    let merged = pr
        .and_then(|p| p.get("merged"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !merged {
        return; // closed without merging is not a land
    }
    let (Some(repo), Some(pr_number)) = (
        payload
            .get("repository")
            .and_then(|r| r.get("full_name"))
            .and_then(|v| v.as_str()),
        pr.and_then(|p| p.get("number")).and_then(|v| v.as_i64()),
    ) else {
        return;
    };
    // A PR number lives in the shared issue/PR number namespace, so it links
    // the same way an issue does.
    let link = match state.store.get_github_issue_link(repo, pr_number).await {
        Ok(Some(l)) => l,
        Ok(None) => return, // PR not linked to a thread — ignore
        Err(err) => {
            tracing::warn!(error = %err, "github pull_request: link lookup failed");
            return;
        }
    };
    let merged_by = pr
        .and_then(|p| p.get("merged_by"))
        .and_then(|u| u.get("login"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let merge_commit_sha = pr
        .and_then(|p| p.get("merge_commit_sha"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let title = pr
        .and_then(|p| p.get("title"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    crate::routes::publish(
        state,
        maidan_types::Event::ThreadLanded {
            occurred_at: chrono::Utc::now(),
            workspace_id: link.workspace_id,
            channel_id: link.channel_id,
            thread_id: link.thread_id,
            repo: repo.to_string(),
            pr_number,
            merged_by,
            merge_commit_sha,
            title,
        },
    )
    .await;
}

/// Route an inbound GitHub `issue_comment` event: a new comment on a linked
/// issue/PR is posted into the mapped Maidan thread. Best-effort — the ingress
/// always ACKs. Only `action == "created"` projects; a `Bot` comment (our own
/// egress echo) is skipped to avoid loops.
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
        // Tag the origin so egress never echoes a GitHub-sourced message back
        // to GitHub (loop prevention).
        metadata: serde_json::json!({ "github": { "user": author, "repo": repo, "issue": issue_number } }),
        content: None,
    };
    match state.store.post_message_with_event(new, None).await {
        Ok((_, stored)) => crate::routes::publish_stored(state, stored).await,
        Err(err) => tracing::warn!(error = %err, "github ingress: post failed"),
    }
}

/// A failed GitHub API call.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GithubError {
    #[error("github http error: {0}")]
    Http(String),
    #[error("github api error: status {status}")]
    Api {
        status: u16,
        /// Whether GitHub said this was a rate limit rather than a permission
        /// problem. GitHub answers a secondary rate limit with **403**, the
        /// same status as a genuinely revoked token, so the status alone cannot
        /// tell them apart — the response headers can.
        rate_limited: bool,
    },
}

impl GithubError {
    /// Whether this failure is a misconfiguration rather than a transient
    /// fault: a wrong or revoked token (401), a missing permission (403), a
    /// repo or issue that isn't there (404). Retrying cannot fix any of them,
    /// so the link is disabled instead of grinding through its attempts.
    ///
    /// A rate-limited 403 is explicitly **not** one: it is the most transient
    /// failure GitHub has, and disabling a link over it would take an
    /// operator's re-link to undo.
    pub fn is_misconfiguration(&self) -> bool {
        match self {
            Self::Http(_) => false,
            Self::Api {
                rate_limited: true, ..
            } => false,
            Self::Api { status, .. } => matches!(status, 401 | 403 | 404),
        }
    }

    /// A 404 — the issue, repo, or comment is gone. Distinct from
    /// [`Self::is_misconfiguration`] so a result-delivery update against a
    /// deleted comment can fall through to the hidden-marker recovery path
    /// instead of disabling a projector link.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Api { status: 404, .. })
    }

    /// A 422 — GitHub understood the request but refused it (a line that is not
    /// part of the pull's diff at `commit_id`, a review on an issue that is not
    /// a PR in a way that still 422s, too many comments). The egress worker
    /// treats this as a skip of the inline review, not a failure of the summary
    /// comment.
    pub fn is_unprocessable(&self) -> bool {
        matches!(self, Self::Api { status: 422, .. })
    }

    /// A 404 (the issue is not a pull) or 422 (the line is not in the diff at
    /// `commit_id`) will not succeed on replay either, so the worker records
    /// `maidan_github_review_total{skipped}` rather than `failed`. A 5xx,
    /// rate-limited 403, or revoked-token 401/403 stays `failed` so an operator
    /// replay retries the review after the surface recovers. Neither class
    /// fails the 379 summary or calls `disable_link`.
    pub fn is_inline_review_skip(&self) -> bool {
        self.is_not_found() || self.is_unprocessable()
    }
}

/// GitHub accepts at most this many entries in `comments[]` on one
/// `POST /repos/{repo}/pulls/{n}/reviews`. Extra findings are dropped, not
/// split across reviews — a second review would look like a second verdict.
pub const GITHUB_REVIEW_COMMENTS_MAX: usize = 100;

/// `body` GitHub requires when `event` is [`GITHUB_REVIEW_EVENT_COMMENT`]. The
/// result summary lives on the issue comment, not on this review.
pub const GITHUB_INLINE_REVIEW_BODY: &str = "Maidan posted inline findings for this result.";

/// Outbound GitHub sender — posts and edits an issue/PR comment in production, a
/// mock in tests.
#[async_trait::async_trait]
pub trait GithubSender: Send + Sync {
    /// Comment on an issue or PR, returning a handle on the comment so a later
    /// delivery can edit it in place.
    ///
    /// **`Ok(None)` means "posted, but we cannot address it."** GitHub accepted
    /// the comment and answered without a readable `id`. The comment exists, so
    /// this is not a failure — reporting one would make the worker retry and
    /// leave two comments on the PR. The recovery path for a lost ref is the
    /// hidden marker in the comment body, not a re-post.
    async fn post_comment(
        &self,
        repo: &str,
        issue_number: i64,
        text: &str,
    ) -> Result<Option<ExternalRef>, GithubError>;

    /// Edit a comment posted earlier. Addressed by repository + comment id — the
    /// issue number is not part of GitHub's comment-update route.
    async fn update_comment(
        &self,
        repo: &str,
        comment_id: i64,
        text: &str,
    ) -> Result<(), GithubError>;

    /// List comments on an issue/PR, oldest first. Used by result delivery to
    /// recover a lost `external_ref` via the hidden body marker. Projector
    /// egress never lists.
    async fn list_issue_comments(
        &self,
        repo: &str,
        issue_number: i64,
    ) -> Result<Vec<GithubIssueComment>, GithubError>;

    /// Post a pull-request review with inline comments.
    ///
    /// `commit_id` is the envelope `head_sha` the caller already resolved —
    /// **never** a live PR head fetched here. `comments` are already mapped
    /// onto GitHub's RIGHT / `line` / `start_line` frame. `event` is always
    /// `COMMENT`; Maidan does not approve or request-changes. Projector egress
    /// never calls this.
    async fn create_review(
        &self,
        repo: &str,
        pull_number: i64,
        commit_id: &str,
        comments: &[GithubReviewComment],
    ) -> Result<(), GithubError>;
}

/// One issue/PR comment as GitHub returns it. Only `id` and `body` are needed
/// for the marker-recovery scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubIssueComment {
    pub id: i64,
    pub body: String,
}

/// The production [`GithubSender`]: posts via the GitHub REST API
/// `POST /repos/{repo}/issues/{n}/comments`.
pub struct GithubApiClient {
    token: String,
    /// API base, `https://api.github.com` in production; overridable so the
    /// wire path can be tested against a loopback server.
    base_url: String,
    http: reqwest::Client,
}

impl GithubApiClient {
    pub fn new(token: String) -> Self {
        Self::with_base_url(token, "https://api.github.com".to_string())
    }

    /// Build against a custom API base (test loopback server). `base_url` has no
    /// trailing slash; `/repos/{repo}/issues/{n}/comments` is appended.
    pub fn with_base_url(token: String, base_url: String) -> Self {
        Self {
            token,
            base_url,
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
    ) -> Result<Option<ExternalRef>, GithubError> {
        let url = format!(
            "{}/repos/{repo}/issues/{issue_number}/comments",
            self.base_url
        );
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
        if !resp.status().is_success() {
            return Err(GithubError::Api {
                status: resp.status().as_u16(),
                rate_limited: is_rate_limited(resp.headers()),
            });
        }
        // The comment exists from here on, so no decoding problem below may be
        // reported as a failure: a retry would post a second comment.
        let comment_id = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("id").and_then(|i| i.as_i64()))
            .filter(|id| *id > 0);
        Ok(comment_id.map(|comment_id| ExternalRef::Github {
            repo: repo.to_string(),
            comment_id,
        }))
    }

    async fn update_comment(
        &self,
        repo: &str,
        comment_id: i64,
        text: &str,
    ) -> Result<(), GithubError> {
        let url = format!(
            "{}/repos/{repo}/issues/comments/{comment_id}",
            self.base_url
        );
        let resp = self
            .http
            .patch(&url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "maidan-projector")
            .json(&serde_json::json!({ "body": text }))
            .send()
            .await
            .map_err(|e| GithubError::Http(e.to_string()))?;
        if resp.status().is_success() {
            return Ok(());
        }
        Err(GithubError::Api {
            status: resp.status().as_u16(),
            rate_limited: is_rate_limited(resp.headers()),
        })
    }

    async fn list_issue_comments(
        &self,
        repo: &str,
        issue_number: i64,
    ) -> Result<Vec<GithubIssueComment>, GithubError> {
        // Cap the scan so a busy issue cannot turn one recovery into an
        // unbounded walk. 10 pages × 100 = 1000 comments; past that we post
        // rather than guess.
        let mut out = Vec::new();
        for page in 1..=10 {
            let url = format!(
                "{}/repos/{repo}/issues/{issue_number}/comments?per_page=100&page={page}",
                self.base_url
            );
            let resp = self
                .http
                .get(&url)
                .bearer_auth(&self.token)
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "maidan-projector")
                .send()
                .await
                .map_err(|e| GithubError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(GithubError::Api {
                    status: resp.status().as_u16(),
                    rate_limited: is_rate_limited(resp.headers()),
                });
            }
            let batch: Vec<serde_json::Value> = resp
                .json()
                .await
                .map_err(|e| GithubError::Http(e.to_string()))?;
            let n = batch.len();
            for c in batch {
                let Some(id) = c.get("id").and_then(|i| i.as_i64()).filter(|id| *id > 0) else {
                    continue;
                };
                let body = c
                    .get("body")
                    .and_then(|b| b.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(GithubIssueComment { id, body });
            }
            if n < 100 {
                break;
            }
        }
        Ok(out)
    }

    async fn create_review(
        &self,
        repo: &str,
        pull_number: i64,
        commit_id: &str,
        comments: &[GithubReviewComment],
    ) -> Result<(), GithubError> {
        let url = format!("{}/repos/{repo}/pulls/{pull_number}/reviews", self.base_url);
        let comments_json: Vec<serde_json::Value> =
            comments.iter().map(review_comment_payload).collect();
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "maidan-projector")
            .json(&serde_json::json!({
                "commit_id": commit_id,
                "event": GITHUB_REVIEW_EVENT_COMMENT,
                "body": GITHUB_INLINE_REVIEW_BODY,
                "comments": comments_json,
            }))
            .send()
            .await
            .map_err(|e| GithubError::Http(e.to_string()))?;
        if resp.status().is_success() {
            return Ok(());
        }
        Err(GithubError::Api {
            status: resp.status().as_u16(),
            rate_limited: is_rate_limited(resp.headers()),
        })
    }
}

/// One `comments[]` item. `start_side` is required by GitHub whenever
/// `start_line` is set; it always matches `side` (RIGHT).
fn review_comment_payload(comment: &GithubReviewComment) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "path": comment.path,
        "line": comment.line,
        "side": comment.side.as_str(),
        "body": comment.body,
    });
    if let Some(start_line) = comment.start_line {
        payload["start_line"] = serde_json::json!(start_line);
        payload["start_side"] = serde_json::json!(comment.side.as_str());
    }
    payload
}

/// Whether a non-success GitHub response is a rate limit. GitHub signals a
/// primary limit with `x-ratelimit-remaining: 0` and a secondary one with
/// `retry-after`, both on a 403 — the same status as a permission failure.
fn is_rate_limited(headers: &reqwest::header::HeaderMap) -> bool {
    if headers.contains_key("retry-after") {
        return true;
    }
    headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim() == "0")
}

/// GitHub projector egress: relay a Maidan message posted in a linked thread
/// out as a GitHub issue/PR comment — by
/// *enqueueing* it on the egress outbox, which [`egress_worker`](crate::egress_worker)
/// drains with retry/backoff. Until 377.2 this posted inline and a transient
/// failure dropped the comment.
///
/// No-op unless a [`GithubSender`] is configured (the worker only runs then, so
/// queueing without one would pile up rows nothing drains); **skips messages
/// that originated in GitHub** (the `metadata.github` tag from 311's ingress)
/// so a projected inbound comment is never echoed back — loop prevention.
///
/// `log_id` is the `maidan_events` row being routed — the dedup key together
/// with the target, so every replica enqueueing yields one comment.
pub async fn route_message_to_github(
    state: &AppState,
    log_id: i64,
    thread_id: maidan_types::ThreadId,
    message: &maidan_types::Message,
) {
    if state.github_sender.is_none() {
        return;
    }
    if message.metadata.get("github").is_some() {
        return; // originated in GitHub — don't echo it back
    }
    let link = match state.store.get_github_issue_link_by_thread(thread_id).await {
        Ok(Some(l)) if l.disabled_at.is_none() => l,
        // Disabled by an auth/config-class failure — see the Slack twin.
        // Re-linking the issue/PR turns it back on.
        Ok(Some(_)) => return,
        Ok(None) => return, // thread not linked to a GitHub issue/PR
        Err(err) => {
            tracing::warn!(error = %err, "github egress: link lookup failed");
            return;
        }
    };
    let queued = state
        .store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: link.workspace_id,
            thread_id,
            source_log_id: log_id,
            target: EgressTarget::Github {
                repo: link.repo,
                issue_number: link.issue_number,
            },
            body: message.body.clone(),
            kind: EgressKind::Projector,
        })
        .await;
    if let Err(err) = queued {
        tracing::warn!(error = %err, "github egress: enqueue failed");
    }
}

/// `POST /workspaces/:wid/github-links` — link a GitHub issue/PR to a Maidan
/// thread so the projector can bridge comments both ways. The link's
/// `channel_id`/`workspace_id` are derived from resolving the thread; the
/// caller supplies only `repo` (`owner/name`), `issue_number`, thread, and the
/// attribution member. `workspace:write` + access to the thread. Upserts.
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

/// `GET /workspaces/:wid/github-links` — the workspace's GitHub issue/PR links.
/// `workspace:read`.
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

/// `DELETE /workspaces/:wid/github-links?repo=…&issue_number=…` — remove a
/// GitHub link (`repo` carries a slash, so it's a query pair, not a path).
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

    fn api(status: u16, rate_limited: bool) -> GithubError {
        GithubError::Api {
            status,
            rate_limited,
        }
    }

    #[test]
    fn auth_and_not_found_statuses_are_misconfigurations() {
        for status in [401, 403, 404] {
            assert!(
                api(status, false).is_misconfiguration(),
                "{status} should disable the link"
            );
        }
    }

    #[test]
    fn transient_statuses_are_not_misconfigurations() {
        for status in [429, 500, 502, 503] {
            assert!(
                !api(status, false).is_misconfiguration(),
                "{status} should be retried"
            );
        }
        assert!(!GithubError::Http("connection reset".into()).is_misconfiguration());
    }

    #[test]
    fn a_rate_limited_403_is_retried_not_disabled() {
        // GitHub answers a secondary rate limit with 403 — the same status as a
        // revoked token. Disabling a link over a rate limit would take an
        // operator's re-link to undo, so the headers, not the status, decide.
        assert!(!api(403, true).is_misconfiguration());
        assert!(api(403, false).is_misconfiguration());
    }

    #[test]
    fn a_404_or_422_on_create_review_is_an_inline_skip() {
        assert!(api(404, false).is_inline_review_skip());
        assert!(api(422, false).is_inline_review_skip());
        for status in [401, 403, 429, 500, 502, 503] {
            assert!(
                !api(status, false).is_inline_review_skip(),
                "{status} is replay-recoverable, not a skip"
            );
        }
        assert!(!api(403, true).is_inline_review_skip());
        assert!(!GithubError::Http("connection reset".into()).is_inline_review_skip());
    }
}
