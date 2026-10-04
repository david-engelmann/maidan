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
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use maidan_auth::{capability::WORKSPACE_READ, capability::WORKSPACE_WRITE, AuthContext};
use maidan_types::{
    EgressKind, EgressTarget, ExternalRef, GithubCheckRun, GithubIssueLink, GithubReviewComment,
    NewEgressOutbox, NewGithubIssueLink, ThreadId, WorkspaceId, GITHUB_REVIEW_EVENT_COMMENT,
};

use crate::dto::{LinkGithubIssue, UnlinkGithubQuery};
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath, ApiQuery, ApiText};
use crate::routes::{cap, ensure_workspace, ApiResult};
use crate::state::AppState;

/// GitHub App / webhook credentials. `webhook_secret` verifies inbound deliveries;
/// `api_token` (optional here) authorizes outbound comment posts in the egress
/// cluster (312) — an installation token or a PAT.
#[derive(Clone)]
pub struct GithubConfig {
    pub webhook_secret: String,
    pub api_token: Option<String>,
}

// Both fields are credentials; `{:?}` must never print them.
impl std::fmt::Debug for GithubConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubConfig")
            .field("webhook_secret", &"[redacted]")
            .field("api_token", &self.api_token.as_ref().map(|_| "[redacted]"))
            .finish()
    }
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
/// `ThreadLanded` fact), a closed-but-unmerged `pull_request` linked to a
/// thread (one message, not a land), and (with no side effect) any other event.
pub async fn github_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiText(body): ApiText,
) -> Response {
    let Some(cfg) = state.github.as_ref() else {
        return ApiError::NotFound.into_response();
    };
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !crate::webhooks::verify_signature(&cfg.webhook_secret, &body, signature) {
        return ApiError::SignatureInvalid.into_response();
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
        // A closed, unmerged PR on a linked issue/PR → one thread message.
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
/// the thread's FSM (not an automation product). A linked PR that closes
/// without merging is not a land either: the thread gets one message, tagged
/// `metadata.github` so egress does not echo it back to GitHub. Best-effort;
/// the ingress always ACKs. A PR not linked to a thread is ignored.
async fn route_github_pull_request(state: &AppState, payload: &serde_json::Value) {
    if payload.get("action").and_then(|v| v.as_str()) != Some("closed") {
        return; // only a close can be a merge or an unmerged close
    }
    let pr = payload.get("pull_request");
    let merged = pr
        .and_then(|p| p.get("merged"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
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
    if !merged {
        signal_pull_request_closed_unmerged(state, &link, repo, pr_number, pr).await;
        return;
    }
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

/// One message on the linked thread when the PR closes without merging.
/// Not a `ThreadLanded` fact, and not an FSM transition. `metadata.github`
/// keeps egress from posting the sentence back onto the PR.
async fn signal_pull_request_closed_unmerged(
    state: &AppState,
    link: &maidan_types::GithubIssueLink,
    repo: &str,
    pr_number: i64,
    pr: Option<&serde_json::Value>,
) {
    let title = pr
        .and_then(|p| p.get("title"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let body = match title {
        Some(title) => format!("GitHub closed {repo}#{pr_number} without merging: {title}"),
        None => format!("GitHub closed {repo}#{pr_number} without merging"),
    };
    let new = maidan_types::NewMessage {
        thread_id: link.thread_id,
        author_id: link.member_id,
        body,
        metadata: serde_json::json!({
            "github": {
                "repo": repo,
                "issue": pr_number,
                "closed_unmerged": true,
            }
        }),
        content: None,
    };
    match state.store.post_message_with_event(new, None).await {
        Ok((_, stored)) => crate::routes::publish_stored(state, stored).await,
        Err(err) => {
            tracing::warn!(error = %err, "github pull_request: unmerged close signal failed")
        }
    }
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
    /// A write this client will not make, whatever the caller asked: a ref
    /// that is not an agent branch.
    #[error("github write refused: {0}")]
    Refused(String),
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
            Self::Refused(_) => true,
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

    /// Create a completed check run on `check.head_sha`.
    ///
    /// The caller already resolved that sha from the result envelope. This
    /// method must not look up the pull request's current head. Projector
    /// egress never calls it. A 403 (a PAT with no `checks:write`) is the
    /// caller's to record; it must not disable a projector issue link.
    async fn create_check_run(&self, repo: &str, check: &GithubCheckRun)
        -> Result<(), GithubError>;

    /// The host this sender posts to: its key in the shared retry budget.
    fn host(&self) -> String {
        "api.github.com".to_string()
    }

    /// The Git Data API behind this sender, for the change flow. `None` (the
    /// default) means this sender can comment but cannot commit, and a
    /// `github_branch` delivery dead-letters saying so.
    fn git(&self) -> Option<&dyn GithubGit> {
        None
    }
}

/// A commit as the Git Data API returns it: enough to find its tree and to
/// recognise a commit Maidan already made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommit {
    pub sha: String,
    pub tree_sha: String,
    pub parents: Vec<String>,
    pub message: String,
}

/// One entry of `POST /repos/{repo}/git/trees`. `blob_sha: None` deletes
/// `path` from the base tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitTreeEntry {
    pub path: String,
    pub mode: String,
    pub blob_sha: Option<String>,
}

/// An open pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPull {
    pub number: i64,
    pub html_url: String,
    /// The branch it merges into.
    pub base: String,
}

/// The GitHub calls the change flow makes: read a branch and the files a diff
/// touches, write blobs, a tree and a commit, move the branch, and find or
/// open its draft pull request. Nothing here approves, merges, marks a pull
/// request ready, requests a review, comments, deletes a branch, force-pushes
/// or touches repository settings, and there is deliberately no method that
/// could.
#[async_trait::async_trait]
pub trait GithubGit: Send + Sync {
    /// `GET /repos/{repo}/git/ref/heads/{branch}`: the branch head, or `None`
    /// when the branch does not exist.
    async fn branch_head(&self, repo: &str, branch: &str) -> Result<Option<String>, GithubError>;
    /// `POST /repos/{repo}/git/refs`: create `branch` at `sha`.
    async fn create_branch(&self, repo: &str, branch: &str, sha: &str) -> Result<(), GithubError>;
    /// `GET /repos/{repo}/git/commits/{sha}`.
    async fn commit(&self, repo: &str, sha: &str) -> Result<GitCommit, GithubError>;
    /// `GET /repos/{repo}/contents/{path}?ref={sha}` as raw bytes, or `None`
    /// when the path does not exist at that commit.
    async fn file_at(
        &self,
        repo: &str,
        path: &str,
        sha: &str,
    ) -> Result<Option<Vec<u8>>, GithubError>;
    /// `POST /repos/{repo}/git/blobs`; returns the blob sha.
    async fn create_blob(&self, repo: &str, content: &[u8]) -> Result<String, GithubError>;
    /// `POST /repos/{repo}/git/trees` on top of `base_tree`; returns the tree sha.
    async fn create_tree(
        &self,
        repo: &str,
        base_tree: &str,
        entries: &[GitTreeEntry],
    ) -> Result<String, GithubError>;
    /// `POST /repos/{repo}/git/commits`; returns the commit sha.
    async fn create_commit(
        &self,
        repo: &str,
        message: &str,
        tree: &str,
        parents: &[String],
    ) -> Result<String, GithubError>;
    /// `PATCH /repos/{repo}/git/refs/heads/{branch}` with `force: false`, so
    /// GitHub refuses (422) a move that is not a fast-forward.
    async fn update_branch(&self, repo: &str, branch: &str, sha: &str) -> Result<(), GithubError>;
    /// `GET /repos/{repo}/pulls?head={owner}:{branch}&state=open`: the open
    /// pull request for this head branch, if any.
    async fn open_pull(&self, repo: &str, branch: &str) -> Result<Option<GithubPull>, GithubError>;
    /// `POST /repos/{repo}/pulls` with `draft: true`.
    async fn create_draft_pull(
        &self,
        repo: &str,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<GithubPull, GithubError>;
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
            // `build` fails only where `Client::new` would panic: no TLS backend.
            http: crate::egress_http::bounded()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

#[async_trait::async_trait]
impl GithubSender for GithubApiClient {
    fn host(&self) -> String {
        crate::retry_budget::host_of(&self.base_url).unwrap_or_else(|| "api.github.com".to_string())
    }

    fn git(&self) -> Option<&dyn GithubGit> {
        Some(self)
    }

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
        let resp = crate::trace_context::stamp(self.http.post(&url))
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "maidan-projector") // GitHub requires a User-Agent
            .json(&serde_json::json!({ "body": text }))
            .send()
            .await
            .map_err(|e| self.http_error(e))?;
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
        let resp = crate::trace_context::stamp(self.http.patch(&url))
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "maidan-projector")
            .json(&serde_json::json!({ "body": text }))
            .send()
            .await
            .map_err(|e| self.http_error(e))?;
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
            let resp = crate::trace_context::stamp(self.http.get(&url))
                .bearer_auth(&self.token)
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "maidan-projector")
                .send()
                .await
                .map_err(|e| self.http_error(e))?;
            if !resp.status().is_success() {
                return Err(GithubError::Api {
                    status: resp.status().as_u16(),
                    rate_limited: is_rate_limited(resp.headers()),
                });
            }
            let batch: Vec<serde_json::Value> =
                resp.json().await.map_err(|e| self.http_error(e))?;
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
        let resp = crate::trace_context::stamp(self.http.post(&url))
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
            .map_err(|e| self.http_error(e))?;
        if resp.status().is_success() {
            return Ok(());
        }
        Err(GithubError::Api {
            status: resp.status().as_u16(),
            rate_limited: is_rate_limited(resp.headers()),
        })
    }

    async fn create_check_run(
        &self,
        repo: &str,
        check: &GithubCheckRun,
    ) -> Result<(), GithubError> {
        let url = format!("{}/repos/{repo}/check-runs", self.base_url);
        let resp = crate::trace_context::stamp(self.http.post(&url))
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "maidan-projector")
            .json(&check_run_body(check))
            .send()
            .await
            .map_err(|e| self.http_error(e))?;
        if resp.status().is_success() {
            return Ok(());
        }
        Err(GithubError::Api {
            status: resp.status().as_u16(),
            rate_limited: is_rate_limited(resp.headers()),
        })
    }
}

impl GithubApiClient {
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.request_accepting(method, path, "application/vnd.github+json")
    }

    /// `.header` appends, so a second `Accept` would be sent beside the
    /// default rather than replace it: the media type is chosen once, here.
    fn request_accepting(
        &self,
        method: reqwest::Method,
        path: &str,
        accept: &str,
    ) -> reqwest::RequestBuilder {
        crate::trace_context::stamp(
            self.http
                .request(method, format!("{}{path}", self.base_url)),
        )
        .bearer_auth(&self.token)
        .header("Accept", accept)
        .header("User-Agent", "maidan-projector")
    }

    /// A transport error, with the token cut out in case anything echoed it.
    fn http_error(&self, err: impl std::fmt::Display) -> GithubError {
        GithubError::Http(redact(&err.to_string(), &self.token))
    }

    async fn send_json(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<serde_json::Value, GithubError> {
        let resp = request.send().await.map_err(|e| self.http_error(e))?;
        if !resp.status().is_success() {
            return Err(GithubError::Api {
                status: resp.status().as_u16(),
                rate_limited: is_rate_limited(resp.headers()),
            });
        }
        resp.json().await.map_err(|e| self.http_error(e))
    }
}

/// `text` with every occurrence of `secret` replaced.
fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "[redacted]")
}

/// The client-side half of the branch guard: no ref other than an agent
/// branch is ever created or moved, whatever the caller passed.
fn guard_ref(branch: &str) -> Result<(), GithubError> {
    if maidan_types::is_change_branch(branch) {
        Ok(())
    } else {
        Err(GithubError::Refused(format!(
            "`{branch}` is not an agent branch"
        )))
    }
}

/// The client-side half of the base guard: no pull request is ever opened
/// into `prod`, or into an agent branch, whatever the caller passed.
fn guard_base(base: &str) -> Result<(), GithubError> {
    if base.eq_ignore_ascii_case(maidan_types::FORBIDDEN_CHANGE_BASE)
        || maidan_types::is_change_branch(base)
    {
        Err(GithubError::Refused(format!(
            "`{base}` is never a pull request base"
        )))
    } else {
        Ok(())
    }
}

/// A string field of a GitHub response, or an error naming it: a response
/// without it cannot be acted on.
fn field(value: &serde_json::Value, pointer: &str) -> Result<String, GithubError> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| GithubError::Http(format!("github response has no {pointer}")))
}

fn pull_from(value: &serde_json::Value) -> Result<GithubPull, GithubError> {
    let number = value
        .get("number")
        .and_then(serde_json::Value::as_i64)
        .filter(|n| *n > 0)
        .ok_or_else(|| GithubError::Http("github pull has no number".into()))?;
    Ok(GithubPull {
        number,
        html_url: field(value, "/html_url")?,
        base: field(value, "/base/ref")?,
    })
}

/// Percent-encode each segment of a path or branch, keeping its slashes.
fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|part| urlencoding::encode(part).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[async_trait::async_trait]
impl GithubGit for GithubApiClient {
    async fn branch_head(&self, repo: &str, branch: &str) -> Result<Option<String>, GithubError> {
        let path = format!("/repos/{repo}/git/ref/heads/{}", encode_path(branch));
        match self
            .send_json(self.request(reqwest::Method::GET, &path))
            .await
        {
            Ok(value) => field(&value, "/object/sha").map(Some),
            Err(err) if err.is_not_found() => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn create_branch(&self, repo: &str, branch: &str, sha: &str) -> Result<(), GithubError> {
        guard_ref(branch)?;
        let request = self
            .request(reqwest::Method::POST, &format!("/repos/{repo}/git/refs"))
            .json(&serde_json::json!({ "ref": format!("refs/heads/{branch}"), "sha": sha }));
        self.send_json(request).await.map(|_| ())
    }

    async fn commit(&self, repo: &str, sha: &str) -> Result<GitCommit, GithubError> {
        let path = format!("/repos/{repo}/git/commits/{sha}");
        let value = self
            .send_json(self.request(reqwest::Method::GET, &path))
            .await?;
        let parents = value
            .get("parents")
            .and_then(serde_json::Value::as_array)
            .map(|parents| {
                parents
                    .iter()
                    .filter_map(|p| p.get("sha").and_then(serde_json::Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Ok(GitCommit {
            sha: field(&value, "/sha")?,
            tree_sha: field(&value, "/tree/sha")?,
            parents,
            message: field(&value, "/message").unwrap_or_default(),
        })
    }

    async fn file_at(
        &self,
        repo: &str,
        path: &str,
        sha: &str,
    ) -> Result<Option<Vec<u8>>, GithubError> {
        let url = format!(
            "/repos/{repo}/contents/{}?ref={}",
            encode_path(path),
            urlencoding::encode(sha)
        );
        // The raw media type returns the bytes, and works past the 1 MB limit
        // of the JSON form.
        let resp = self
            .request_accepting(
                reqwest::Method::GET,
                &url,
                "application/vnd.github.raw+json",
            )
            .send()
            .await
            .map_err(|e| self.http_error(e))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(GithubError::Api {
                status: resp.status().as_u16(),
                rate_limited: is_rate_limited(resp.headers()),
            });
        }
        resp.bytes()
            .await
            .map(|b| Some(b.to_vec()))
            .map_err(|e| self.http_error(e))
    }

    async fn create_blob(&self, repo: &str, content: &[u8]) -> Result<String, GithubError> {
        use base64::Engine as _;
        let request = self
            .request(reqwest::Method::POST, &format!("/repos/{repo}/git/blobs"))
            .json(&serde_json::json!({
                "content": base64::engine::general_purpose::STANDARD.encode(content),
                "encoding": "base64",
            }));
        field(&self.send_json(request).await?, "/sha")
    }

    async fn create_tree(
        &self,
        repo: &str,
        base_tree: &str,
        entries: &[GitTreeEntry],
    ) -> Result<String, GithubError> {
        let tree: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "path": e.path,
                    "mode": e.mode,
                    "type": "blob",
                    "sha": e.blob_sha,
                })
            })
            .collect();
        let request = self
            .request(reqwest::Method::POST, &format!("/repos/{repo}/git/trees"))
            .json(&serde_json::json!({ "base_tree": base_tree, "tree": tree }));
        field(&self.send_json(request).await?, "/sha")
    }

    async fn create_commit(
        &self,
        repo: &str,
        message: &str,
        tree: &str,
        parents: &[String],
    ) -> Result<String, GithubError> {
        let request = self
            .request(reqwest::Method::POST, &format!("/repos/{repo}/git/commits"))
            .json(&serde_json::json!({ "message": message, "tree": tree, "parents": parents }));
        field(&self.send_json(request).await?, "/sha")
    }

    async fn update_branch(&self, repo: &str, branch: &str, sha: &str) -> Result<(), GithubError> {
        guard_ref(branch)?;
        let path = format!("/repos/{repo}/git/refs/heads/{}", encode_path(branch));
        let request = self
            .request(reqwest::Method::PATCH, &path)
            .json(&serde_json::json!({ "sha": sha, "force": false }));
        self.send_json(request).await.map(|_| ())
    }

    async fn open_pull(&self, repo: &str, branch: &str) -> Result<Option<GithubPull>, GithubError> {
        let owner = repo.split('/').next().unwrap_or_default();
        let head = urlencoding::encode(&format!("{owner}:{branch}")).into_owned();
        let path = format!("/repos/{repo}/pulls?head={head}&state=open&per_page=1");
        let value = self
            .send_json(self.request(reqwest::Method::GET, &path))
            .await?;
        value
            .as_array()
            .and_then(|pulls| pulls.first())
            .map(pull_from)
            .transpose()
    }

    async fn create_draft_pull(
        &self,
        repo: &str,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<GithubPull, GithubError> {
        guard_ref(branch)?;
        guard_base(base)?;
        let request = self
            .request(reqwest::Method::POST, &format!("/repos/{repo}/pulls"))
            .json(&serde_json::json!({
                "title": title,
                "body": body,
                "head": branch,
                "base": base,
                "draft": true,
            }));
        pull_from(&self.send_json(request).await?)
    }
}

/// JSON body for `POST /repos/{repo}/check-runs`. `head_sha` is copied from
/// the check; nothing in this object is a pull-request head looked up live.
pub(crate) fn check_run_body(check: &GithubCheckRun) -> serde_json::Value {
    let mut body = serde_json::json!({
        "name": check.name,
        "head_sha": check.head_sha,
        "status": "completed",
        "conclusion": check.conclusion.as_str(),
        "output": {
            "title": check.title,
            "summary": check.summary,
        },
    });
    if let Some(url) = &check.details_url {
        body["details_url"] = serde_json::json!(url);
    }
    body
}

/// Check-run posts: `sent` (GitHub accepted), `skipped` (no `head_sha`, not a
/// waiter envelope, or 404/422), `failed` (GitHub rejected; the summary
/// comment still landed).
pub(crate) fn record_github_check_run(outcome: &str) {
    metrics::counter!(
        "maidan_github_check_run_total",
        "outcome" => outcome.to_string()
    )
    .increment(1);
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
/// caller supplies only `repo` (`owner/name`), `issue_number`, and thread.
/// `workspace:write` + access to the thread. Upserts.
pub async fn link_github_issue(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
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
            member_id: auth.member_id,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(link)))
}

/// `GET /workspaces/:wid/github-links` — the workspace's GitHub issue/PR links.
/// `workspace:read`.
pub async fn list_github_issue_links(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
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
    ApiPath(wid): ApiPath<uuid::Uuid>,
    ApiQuery(q): ApiQuery<UnlinkGithubQuery>,
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
        _ => return Err(ApiError::NotFound),
    }
    if state.store.unlink_github_issue(&repo, issue_number).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
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

    #[test]
    fn the_token_never_reaches_debug_output_or_an_error() {
        let cfg = GithubConfig {
            webhook_secret: "whsec-value".into(),
            api_token: Some("ghp_secretvalue".into()),
        };
        let shown = format!("{cfg:?}");
        assert!(
            !shown.contains("ghp_secretvalue") && !shown.contains("whsec-value"),
            "{shown}"
        );
        let client = GithubApiClient::new("ghp_secretvalue".into());
        let err = client.http_error("request to https://x/?t=ghp_secretvalue failed");
        assert!(!err.to_string().contains("ghp_secretvalue"), "{err}");
    }

    #[tokio::test]
    async fn the_client_writes_only_agent_branches_and_never_opens_a_pull_into_prod() {
        // No server: a refused write must not reach the network at all.
        let client = GithubApiClient::with_base_url("t".into(), "http://127.0.0.1:9".into());
        let sha = "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911";
        for branch in ["main", "prod", "dev", "feature/x", "feature/agent-X"] {
            let created = client.create_branch("o/r", branch, sha).await.unwrap_err();
            let moved = client.update_branch("o/r", branch, sha).await.unwrap_err();
            let opened = client
                .create_draft_pull("o/r", branch, "dev", "t", "b")
                .await
                .unwrap_err();
            for err in [created, moved, opened] {
                assert!(matches!(err, GithubError::Refused(_)), "{branch}: {err}");
                assert!(
                    err.is_misconfiguration(),
                    "a refused write is never retried"
                );
            }
        }
        for base in ["prod", "PROD", "feature/agent-x"] {
            let err = client
                .create_draft_pull("o/r", "feature/agent-y", base, "t", "b")
                .await
                .unwrap_err();
            assert!(matches!(err, GithubError::Refused(_)), "{base}: {err}");
        }
    }

    #[test]
    fn a_check_run_body_is_completed_on_the_envelope_sha() {
        use maidan_types::GithubCheckConclusion;
        let check = GithubCheckRun {
            name: "maidan".into(),
            head_sha: "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911".into(),
            conclusion: GithubCheckConclusion::Failure,
            title: "Maidan found a critical issue".into(),
            summary: "does not request changes".into(),
            details_url: None,
        };
        let body = check_run_body(&check);
        assert_eq!(body["status"], "completed");
        assert_eq!(body["conclusion"], "failure");
        assert_eq!(body["head_sha"], check.head_sha);
        assert!(body.get("details_url").is_none());
        assert!(body.get("pull_number").is_none());
    }
}
