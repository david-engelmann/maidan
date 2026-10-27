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
use crate::routes::{cap, ensure_workspace, ApiResult, MarkReadyGuardPass};
use crate::state::AppState;

/// GitHub App / webhook credentials. `webhook_secret` verifies inbound deliveries;
/// `api_token` (optional here) authorizes outbound comment posts in the egress
/// cluster (312) — an installation token or a PAT.
#[derive(Clone)]
pub struct GithubConfig {
    pub webhook_secret: String,
    pub api_token: Option<String>,
    /// `MAIDAN_GITHUB_WRITE_REPOS`: the only repositories the token may write
    /// to. Not a secret; empty means the token writes nowhere.
    pub write_repos: Vec<String>,
}

// The secret and the token are credentials; `{:?}` must never print them.
impl std::fmt::Debug for GithubConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubConfig")
            .field("webhook_secret", &"[redacted]")
            .field("api_token", &self.api_token.as_ref().map(|_| "[redacted]"))
            .field("write_repos", &self.write_repos)
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
        Some(GithubConfig {
            webhook_secret,
            api_token: api_token_from_env(),
            write_repos: write_repos_from_env(),
        })
    }
}

/// `MAIDAN_GITHUB_TOKEN`, the egress credential when no GitHub App is
/// configured. Read on its own: egress does not need the webhook secret.
pub fn api_token_from_env() -> Option<String> {
    std::env::var("MAIDAN_GITHUB_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

/// `MAIDAN_GITHUB_WRITE_REPOS`: the only repositories GitHub egress may write
/// to.
pub fn write_repos_from_env() -> Vec<String> {
    parse_write_repos(&std::env::var("MAIDAN_GITHUB_WRITE_REPOS").unwrap_or_default())
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
    /// that is not an agent branch. Also a GitHub App token exchange GitHub
    /// refused (a wrong key or a removed installation): no call can succeed
    /// until the operator fixes it.
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
    /// The label names it carries.
    pub labels: Vec<String>,
    /// Its title; empty when the answer omits it.
    pub title: String,
    /// Its description; empty when it has none.
    pub body: String,
}

/// A pull request as read for the mark-ready guard: head, base, draft flag
/// and the GraphQL node id the ready mutation needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullBrief {
    pub number: i64,
    /// The head branch name.
    pub head: String,
    /// The base branch name.
    pub base: String,
    pub draft: bool,
    /// The PR's GraphQL node id, from the same fresh read the guards ran on.
    pub node_id: String,
    /// Whether the head branch lives in the base repository. A fork's
    /// `feature/agent-*` branch is not one of the change flow's, so the flip
    /// refuses it; a head repository GitHub no longer reports (a deleted
    /// fork) reads as `false`.
    pub same_repo: bool,
    /// Whether the pull request is open. A closed draft cannot be marked
    /// ready, so the flip refuses it plainly instead of failing at GitHub.
    pub open: bool,
    /// The head commit, from the same read: the enqueue guard compares it with
    /// the commit Maidan delivered.
    pub head_sha: String,
    /// The label names the pull request carries.
    pub labels: Vec<String>,
    /// When it merged, if it has.
    pub merged_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When it closed, merged or not.
    pub closed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The commit the merge made on the base, once merged: what a revert
    /// names.
    pub merge_commit_sha: Option<String>,
}

/// A commit on a branch, as GitHub lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubCommitBrief {
    pub sha: String,
    pub message: String,
}

/// A merged pull request, as GitHub lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubMergedPull {
    pub number: i64,
    pub title: String,
    pub body: String,
    pub merged_at: chrono::DateTime<chrono::Utc>,
}

/// A review or review comment on a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubReviewActivity {
    /// The author is a bot account (`type: Bot`, or a `[bot]` login).
    pub bot: bool,
    pub at: Option<chrono::DateTime<chrono::Utc>>,
}

/// How one check on a pull request's head stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubCheckState {
    /// Queued or running, or a status still pending.
    Pending,
    /// Finished without failing: success, neutral or skipped.
    Passed,
    /// Finished failing: failure, timed out, cancelled, needing action, or
    /// a status that errored.
    Failed,
}

/// One check run or commit status on a pull request's head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubCheck {
    /// The check run's name, or the status's context.
    pub name: String,
    pub state: GithubCheckState,
    /// The GitHub Actions job behind a check run, whose log can be read;
    /// `None` for a status or another app's check.
    pub job_id: Option<i64>,
}

/// A pull request as the change sweep reads it each tick: what the brief
/// says, and the checks on its head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullSnapshot {
    pub brief: GithubPullBrief,
    /// The checks on the head commit; `None` when this read did not carry
    /// them, empty when nothing has reported yet.
    pub checks: Option<Vec<GithubCheck>>,
    /// GitHub refused this token the head's checks (`FORBIDDEN` on the
    /// rollup). The read still answers, with no checks, so without this the
    /// head would look like one nothing has reported on, forever.
    pub checks_denied: bool,
    /// What else decides whether it can merge; `None` when this read did not
    /// carry it.
    pub gate: Option<GithubGate>,
    /// Whether the head has commits its base lacks; `None` when this read
    /// did not say.
    pub ahead: Option<bool>,
}

/// What GitHub says about whether a pull request can merge, besides its
/// checks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GithubGate {
    /// `mergeStateStatus`: `CLEAN`, `BEHIND`, `BLOCKED`, `DIRTY`, ...
    pub merge_state: Option<String>,
    /// The unresolved review threads among the first 100, bots' included.
    pub unresolved_threads: Vec<maidan_types::ReviewThreadLink>,
    /// The status checks the base branch's protection requires; `None` when
    /// the credential may not read the protection, or there is none.
    pub required_checks: Option<Vec<String>>,
    /// Whether the pull request is in the base branch's merge queue.
    pub queued: bool,
    /// Why it last left the merge queue (`MERGE_CONFLICT`,
    /// `FAILED_CHECKS`, ...), when the last queue event was a removal.
    pub dequeued: Option<String>,
}

/// What flipping a pull request to ready did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkReadyOutcome {
    /// The pull was a draft and is now ready for review.
    Marked {
        number: i64,
        head: String,
        base: String,
    },
    /// The pull was already ready; nothing was written.
    AlreadyReady {
        number: i64,
        head: String,
        base: String,
    },
}

/// The GitHub calls the change flow makes: read a branch and the files a diff
/// touches, write blobs, a tree and a commit, move the branch, find or open
/// its draft pull request, and — the deliberate exceptions —
/// [`GithubGit::set_pull_ready`] and [`GithubGit::force_move_branch`].
/// Nothing else here approves, merges, requests a review, comments, deletes a
/// branch, force-pushes or touches repository settings, and there is
/// deliberately no other method that could.
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
    /// `PATCH /repos/{repo}/git/refs/heads/{branch}` with `force: true`: the
    /// change flow's only non-fast-forward write, for a squash-rebase round.
    /// The branch and the commit come from the `pass` alone, and only the
    /// guard in [`crate::change_flow`] can mint one: it re-reads the head
    /// right before the move and binds the pass to that agent branch and that
    /// commit. A caller cannot name a branch or commit here, so no other path
    /// can force anything.
    async fn force_move_branch(
        &self,
        repo: &str,
        pass: crate::change_flow::ForceMovePass,
    ) -> Result<(), GithubError>;
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
    /// `PATCH /repos/{repo}/pulls/{n}` with a new `body`. The change flow
    /// calls it only for the agent pull request it delivers to, to keep the
    /// branch's current-state section current.
    async fn update_pull_body(
        &self,
        repo: &str,
        pull_number: i64,
        body: &str,
    ) -> Result<(), GithubError>;
    /// `POST /repos/{repo}/issues/{n}/comments`. The change flow's one
    /// comment: after a squash-rebase moves the agent branch, it names the
    /// old and new heads so a reviewer can diff the range the force rewrote.
    async fn comment_on_pull(
        &self,
        repo: &str,
        pull_number: i64,
        body: &str,
    ) -> Result<(), GithubError>;
    /// `GET /repos/{repo}/compare/{base_sha}...{branch}`: how many commits
    /// `branch` has gained since `base_sha`, for the stale-base warning.
    async fn commits_behind(
        &self,
        repo: &str,
        base_sha: &str,
        branch: &str,
    ) -> Result<i64, GithubError>;
    /// `GET /repos/{repo}/pulls/{n}`: head ref, base ref, draft flag and node
    /// id, for the mark-ready guard. A read, so no guard applies; the caller
    /// enforces the change-flow rules before acting on what it returns.
    async fn pull_brief(
        &self,
        repo: &str,
        pull_number: i64,
    ) -> Result<GithubPullBrief, GithubError>;
    /// The change sweep's one read of a pull request per tick: its brief and
    /// the checks on its head, in one GraphQL query. A read. The default
    /// reads the brief alone, with no checks.
    async fn pull_snapshot(
        &self,
        repo: &str,
        pull_number: i64,
    ) -> Result<GithubPullSnapshot, GithubError> {
        Ok(GithubPullSnapshot {
            brief: self.pull_brief(repo, pull_number).await?,
            checks: None,
            checks_denied: false,
            gate: None,
            ahead: None,
        })
    }
    /// `GET /repos/{repo}/compare/{branch}...{sha}`: whether `sha` is on
    /// `branch` already (behind it or identical). A read, for a pull request
    /// closed without merging whose head a stack tool landed. The default
    /// says no, so a client that cannot compare treats every close without a
    /// merge as one.
    async fn commit_on_branch(
        &self,
        _repo: &str,
        _branch: &str,
        _sha: &str,
    ) -> Result<bool, GithubError> {
        Ok(false)
    }
    /// `GET /repos/{repo}/actions/jobs/{job_id}/logs`: the last `max_bytes`
    /// of a GitHub Actions job's log, starting at a line. A read. The
    /// default reads nothing.
    async fn job_log_tail(
        &self,
        _repo: &str,
        _job_id: i64,
        _max_bytes: usize,
    ) -> Result<String, GithubError> {
        Err(GithubError::Http("this client reads no job logs".into()))
    }
    /// `GET /repos/{repo}/commits?sha={branch}&since=…`: up to the newest 100
    /// commits on `branch` since `since`. A read.
    async fn branch_commits_since(
        &self,
        repo: &str,
        branch: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<GithubCommitBrief>, GithubError>;
    /// `GET /repos/{repo}/pulls?state=closed&base={base}`: the 100 most
    /// recently updated closed pull requests into `base`, merged ones only.
    /// A read.
    async fn merged_pulls_into(
        &self,
        repo: &str,
        base: &str,
    ) -> Result<Vec<GithubMergedPull>, GithubError>;
    /// `GET /repos/{repo}/pulls/{n}/commits`: up to 100 commit SHAs. A read.
    async fn pull_commit_shas(
        &self,
        repo: &str,
        pull_number: i64,
    ) -> Result<Vec<String>, GithubError>;
    /// A pull request's submitted reviews and review comments
    /// (`/pulls/{n}/reviews` and `/pulls/{n}/comments`, up to 100 each). A
    /// read.
    async fn pull_review_activity(
        &self,
        repo: &str,
        pull_number: i64,
    ) -> Result<Vec<GithubReviewActivity>, GithubError>;
    /// Flip a draft pull request to ready for review, via the GraphQL
    /// `markPullRequestReadyForReview` mutation keyed on the PR's `node_id`.
    /// GitHub's REST `PATCH /repos/{repo}/pulls/{n}` with `{"draft": false}`
    /// is a silent no-op — it returns 200 and leaves the draft a draft — so
    /// the flip must go through GraphQL, and the mutation's answer must say
    /// `isDraft: false` before the flip counts as done. No read and no
    /// guards here: the caller must have run the single guard path first
    /// ([`crate::routes::flip_pull_ready_guarded`]), which reads the pull
    /// once, evaluates every guard against that read, and only then calls
    /// this. Guarding here on a second read would reintroduce the TOCTOU the
    /// guard path exists to close, so this method trusts its caller.
    ///
    /// The `_pass` is a [`MarkReadyGuardPass`], mintable only inside
    /// `crate::routes::github_ops`: the type system, not convention, keeps
    /// this method off the direct-call path that would bypass the guards.
    ///
    /// The deliberate, tightly-scoped exception to "never marks ready"
    /// (Decisions, 2026-10-06): the mark-ready app holds no GitHub write credential,
    /// so Maidan does the flip — only for `feature/agent-*` heads into
    /// allowlisted bases, never prod, never anything else.
    async fn set_pull_ready(
        &self,
        repo: &str,
        pull_number: i64,
        node_id: &str,
        _pass: MarkReadyGuardPass,
    ) -> Result<(), GithubError>;
    /// Add `label` to a pull request: the label the repository's merge queue
    /// takes ready work from. No guards here, as for [`Self::set_pull_ready`]:
    /// the `_pass` is mintable only by the enqueue guard path
    /// ([`crate::routes::enqueue_pull_guarded`]), which ran every guard on the
    /// same read first.
    async fn add_pull_label(
        &self,
        repo: &str,
        pull_number: i64,
        label: &str,
        _pass: crate::routes::EnqueueGuardPass,
    ) -> Result<(), GithubError>;
    /// Remove `label` from a pull request; `Ok(false)` when it was not there.
    /// Guarded by the dequeue path, as [`Self::add_pull_label`] is by enqueue.
    async fn remove_pull_label(
        &self,
        repo: &str,
        pull_number: i64,
        label: &str,
        _pass: crate::routes::EnqueueGuardPass,
    ) -> Result<bool, GithubError>;
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
    auth: GithubAuth,
    /// API base, `https://api.github.com` in production; overridable so the
    /// wire path can be tested against a loopback server.
    base_url: String,
    http: reqwest::Client,
    /// The repositories this client may write to. The token belongs to the
    /// instance while egress allowlists belong to workspaces, so the operator
    /// names the repositories here (`MAIDAN_GITHUB_WRITE_REPOS`) and no
    /// workspace can point the token anywhere else. A new client writes
    /// nowhere until it is given a list.
    write_repos: WriteRepos,
}

/// How a [`GithubApiClient`] authenticates.
enum GithubAuth {
    /// `MAIDAN_GITHUB_TOKEN`: a personal access token or an installation
    /// token someone else exchanged.
    Token(String),
    /// A GitHub App: a signed JWT exchanged for installation tokens
    /// ([`crate::github_app`]).
    App(std::sync::Arc<crate::github_app::GithubAppAuth>),
}

/// Where a [`GithubApiClient`] may write.
enum WriteRepos {
    /// Only these `owner/name`s, lowercased. Empty means nowhere.
    Listed(std::collections::HashSet<String>),
    /// Anywhere the token reaches: test loopback servers only.
    Any,
}

/// `MAIDAN_GITHUB_WRITE_REPOS`: comma-separated `owner/name`s, compared
/// case-insensitively as GitHub does. Blank entries are ignored.
pub fn parse_write_repos(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|r| r.trim().to_ascii_lowercase())
        .filter(|r| !r.is_empty())
        .collect()
}

impl GithubApiClient {
    pub fn new(token: String) -> Self {
        Self::with_base_url(token, "https://api.github.com".to_string())
    }

    /// Build against a custom API base (test loopback server). `base_url` has no
    /// trailing slash; `/repos/{repo}/issues/{n}/comments` is appended.
    pub fn with_base_url(token: String, base_url: String) -> Self {
        Self::with_auth(GithubAuth::Token(token), base_url)
    }

    /// Authenticate as a GitHub App instead of with a configured token.
    pub fn with_app(app: std::sync::Arc<crate::github_app::GithubAppAuth>) -> Self {
        Self::with_app_and_base_url(app, "https://api.github.com".to_string())
    }

    /// [`Self::with_app`] against a custom API base (test loopback server).
    pub fn with_app_and_base_url(
        app: std::sync::Arc<crate::github_app::GithubAppAuth>,
        base_url: String,
    ) -> Self {
        Self::with_auth(GithubAuth::App(app), base_url)
    }

    fn with_auth(auth: GithubAuth, base_url: String) -> Self {
        Self {
            auth,
            base_url,
            // `build` fails only where `Client::new` would panic: no TLS backend.
            http: crate::egress_http::bounded()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            write_repos: WriteRepos::Listed(Default::default()),
        }
    }

    /// Let this client write to exactly these `owner/name`s.
    pub fn with_write_repos(mut self, repos: impl IntoIterator<Item = String>) -> Self {
        self.write_repos = WriteRepos::Listed(
            repos
                .into_iter()
                .map(|r| r.trim().to_ascii_lowercase())
                .filter(|r| !r.is_empty())
                .collect(),
        );
        self
    }

    /// Let this client write anywhere its token reaches. For loopback test
    /// servers; production builds the client with [`Self::with_write_repos`].
    pub fn with_any_write_repo(mut self) -> Self {
        self.write_repos = WriteRepos::Any;
        self
    }

    /// Refuse a write to a repository the operator did not name, before any
    /// request is made.
    fn ensure_writable(&self, repo: &str) -> Result<(), GithubError> {
        match &self.write_repos {
            WriteRepos::Any => Ok(()),
            WriteRepos::Listed(repos) if repos.contains(&repo.to_ascii_lowercase()) => Ok(()),
            WriteRepos::Listed(_) => Err(GithubError::Refused(format!(
                "`{repo}` is not in MAIDAN_GITHUB_WRITE_REPOS, the repositories this \
                 instance's GitHub token may write to"
            ))),
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
        self.ensure_writable(repo)?;
        let resp = self
            .request(
                reqwest::Method::POST,
                repo,
                &format!("/issues/{issue_number}/comments"),
            )
            .await?
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
        self.ensure_writable(repo)?;
        let resp = self
            .request(
                reqwest::Method::PATCH,
                repo,
                &format!("/issues/comments/{comment_id}"),
            )
            .await?
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
            let resp = self
                .request(
                    reqwest::Method::GET,
                    repo,
                    &format!("/issues/{issue_number}/comments?per_page=100&page={page}"),
                )
                .await?
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
        self.ensure_writable(repo)?;
        let comments_json: Vec<serde_json::Value> =
            comments.iter().map(review_comment_payload).collect();
        let resp = self
            .request(
                reqwest::Method::POST,
                repo,
                &format!("/pulls/{pull_number}/reviews"),
            )
            .await?
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
        self.ensure_writable(repo)?;
        let resp = self
            .request(reqwest::Method::POST, repo, "/check-runs")
            .await?
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
    /// A request to `path` inside `repo`. Anything but a GET writes to `repo`,
    /// so it is refused unless the operator listed `repo`. Every request is
    /// built here or in [`Self::graphql`], and a test holds the HTTP client to
    /// [`Self::build`], so a write method added later cannot miss the check.
    async fn request(
        &self,
        method: reqwest::Method,
        repo: &str,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, GithubError> {
        self.request_accepting(method, repo, path, "application/vnd.github+json")
            .await
    }

    async fn request_accepting(
        &self,
        method: reqwest::Method,
        repo: &str,
        path: &str,
        accept: &str,
    ) -> Result<reqwest::RequestBuilder, GithubError> {
        if method != reqwest::Method::GET {
            self.ensure_writable(repo)?;
        }
        self.build(method, &format!("/repos/{repo}{path}"), accept)
            .await
    }

    /// A GraphQL request about `repo`. The endpoint is not under the
    /// repository's path, so the repository is named here, and a mutation
    /// (`writes`) is checked against the write list as any other write is.
    async fn graphql(
        &self,
        repo: &str,
        writes: bool,
    ) -> Result<reqwest::RequestBuilder, GithubError> {
        if writes {
            self.ensure_writable(repo)?;
        }
        self.build(
            reqwest::Method::POST,
            "/graphql",
            "application/vnd.github+json",
        )
        .await
    }

    /// `.header` appends, so a second `Accept` would be sent beside the
    /// default rather than replace it: the media type is chosen once, here.
    /// The bearer is fetched here too, after the repository check, so a
    /// refused write never costs a token exchange.
    async fn build(
        &self,
        method: reqwest::Method,
        path: &str,
        accept: &str,
    ) -> Result<reqwest::RequestBuilder, GithubError> {
        let bearer = self.bearer().await?;
        Ok(crate::trace_context::stamp(
            self.http
                .request(method, format!("{}{path}", self.base_url)),
        )
        .bearer_auth(bearer)
        .header("Accept", accept)
        .header("User-Agent", "maidan-projector"))
    }

    /// The configured token, or the GitHub App's installation token, cached
    /// until shortly before it expires.
    async fn bearer(&self) -> Result<String, GithubError> {
        match &self.auth {
            GithubAuth::Token(token) => Ok(token.clone()),
            GithubAuth::App(app) => app.installation_token(&self.http, &self.base_url).await,
        }
    }

    /// A transport error, with the token cut out in case anything echoed it.
    fn http_error(&self, err: impl std::fmt::Display) -> GithubError {
        let text = err.to_string();
        GithubError::Http(match &self.auth {
            GithubAuth::Token(token) => redact(&text, token),
            GithubAuth::App(app) => app.redact(&text),
        })
    }

    /// The checks on `head_sha` read from the Actions and commit-status
    /// APIs (Actions and Commit statuses read), for a token GitHub will not
    /// show check runs. Every workflow run is opened to its jobs, so each
    /// check carries the job name a required context names and a failure's
    /// log tail can be fetched. Check runs from apps other than Actions are
    /// not seen this way. A list longer than this reads stays pending.
    async fn actions_checks(
        &self,
        repo: &str,
        head_sha: &str,
    ) -> Result<Vec<GithubCheck>, GithubError> {
        if !maidan_types::is_git_sha(head_sha) {
            return Err(GithubError::Refused(format!(
                "`{head_sha}` is not a commit"
            )));
        }
        let (runs, mut complete) = self
            .paged(
                repo,
                &format!("/actions/runs?head_sha={head_sha}"),
                "workflow_runs",
            )
            .await?;
        let mut jobs = std::collections::HashMap::new();
        for run_id in runs
            .iter()
            .filter_map(|run| run.get("id").and_then(serde_json::Value::as_i64))
        {
            let (run_jobs, all) = self
                .paged(repo, &format!("/actions/runs/{run_id}/jobs"), "jobs")
                .await?;
            complete &= all;
            jobs.insert(run_id, run_jobs);
        }
        let (statuses, all) = self
            .paged(repo, &format!("/commits/{head_sha}/status"), "statuses")
            .await?;
        complete &= all;
        let mut checks = actions_checks_from(&runs, &jobs, &statuses);
        if !complete {
            checks.push(GithubCheck {
                name: "checks beyond the pages read".into(),
                state: GithubCheckState::Pending,
                job_id: None,
            });
        }
        Ok(checks)
    }

    /// The items under `key` of a paged REST list, up to
    /// [`ACTIONS_MAX_PAGES`] pages of 100, and whether that was all of them.
    async fn paged(
        &self,
        repo: &str,
        path: &str,
        key: &str,
    ) -> Result<(Vec<serde_json::Value>, bool), GithubError> {
        let joiner = if path.contains('?') { '&' } else { '?' };
        let mut items = Vec::new();
        for page in 1..=ACTIONS_MAX_PAGES {
            let answer = self
                .send_json(
                    self.request(
                        reqwest::Method::GET,
                        repo,
                        &format!("{path}{joiner}per_page=100&page={page}"),
                    )
                    .await?,
                )
                .await?;
            let batch: Vec<serde_json::Value> =
                list(answer.get(key).unwrap_or(&serde_json::Value::Null))
                    .cloned()
                    .collect();
            let short = batch.len() < 100;
            items.extend(batch);
            let total = answer
                .get("total_count")
                .and_then(serde_json::Value::as_u64);
            if short || total.is_some_and(|total| items.len() as u64 >= total) {
                return Ok((items, true));
            }
        }
        Ok((items, false))
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
pub(crate) fn redact(text: &str, secret: &str) -> String {
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
        labels: label_names(value),
        title: value
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        body: value
            .get("body")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

/// The `labels[].name` of an issue or pull request as GitHub returns it.
fn label_names(value: &serde_json::Value) -> Vec<String> {
    value
        .get("labels")
        .and_then(serde_json::Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| label.get("name").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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
        let path = format!("/git/ref/heads/{}", encode_path(branch));
        match self
            .send_json(self.request(reqwest::Method::GET, repo, &path).await?)
            .await
        {
            Ok(value) => field(&value, "/object/sha").map(Some),
            Err(err) if err.is_not_found() => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn create_branch(&self, repo: &str, branch: &str, sha: &str) -> Result<(), GithubError> {
        self.ensure_writable(repo)?;
        guard_ref(branch)?;
        let request = self
            .request(reqwest::Method::POST, repo, "/git/refs")
            .await?
            .json(&serde_json::json!({ "ref": format!("refs/heads/{branch}"), "sha": sha }));
        self.send_json(request).await.map(|_| ())
    }

    async fn commit(&self, repo: &str, sha: &str) -> Result<GitCommit, GithubError> {
        let path = format!("/git/commits/{sha}");
        let value = self
            .send_json(self.request(reqwest::Method::GET, repo, &path).await?)
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
            "/contents/{}?ref={}",
            encode_path(path),
            urlencoding::encode(sha)
        );
        // The raw media type returns the bytes, and works past the 1 MB limit
        // of the JSON form.
        let resp = self
            .request_accepting(
                reqwest::Method::GET,
                repo,
                &url,
                "application/vnd.github.raw+json",
            )
            .await?
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
        self.ensure_writable(repo)?;
        use base64::Engine as _;
        let request = self
            .request(reqwest::Method::POST, repo, "/git/blobs")
            .await?
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
        self.ensure_writable(repo)?;
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
            .request(reqwest::Method::POST, repo, "/git/trees")
            .await?
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
        self.ensure_writable(repo)?;
        let request = self
            .request(reqwest::Method::POST, repo, "/git/commits")
            .await?
            .json(&serde_json::json!({ "message": message, "tree": tree, "parents": parents }));
        field(&self.send_json(request).await?, "/sha")
    }

    async fn update_branch(&self, repo: &str, branch: &str, sha: &str) -> Result<(), GithubError> {
        self.ensure_writable(repo)?;
        guard_ref(branch)?;
        let path = format!("/git/refs/heads/{}", encode_path(branch));
        let request = self
            .request(reqwest::Method::PATCH, repo, &path)
            .await?
            .json(&serde_json::json!({ "sha": sha, "force": false }));
        self.send_json(request).await.map(|_| ())
    }

    async fn force_move_branch(
        &self,
        repo: &str,
        pass: crate::change_flow::ForceMovePass,
    ) -> Result<(), GithubError> {
        self.ensure_writable(repo)?;
        // The pass already holds an agent branch; checked again here so the
        // one force write never depends on a single check.
        guard_ref(pass.branch())?;
        let path = format!("/git/refs/heads/{}", encode_path(pass.branch()));
        let request = self
            .request(reqwest::Method::PATCH, repo, &path)
            .await?
            .json(&serde_json::json!({ "sha": pass.to(), "force": true }));
        self.send_json(request).await.map(|_| ())
    }

    async fn open_pull(&self, repo: &str, branch: &str) -> Result<Option<GithubPull>, GithubError> {
        let owner = repo.split('/').next().unwrap_or_default();
        let head = urlencoding::encode(&format!("{owner}:{branch}")).into_owned();
        let path = format!("/pulls?head={head}&state=open&per_page=1");
        let value = self
            .send_json(self.request(reqwest::Method::GET, repo, &path).await?)
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
        self.ensure_writable(repo)?;
        guard_ref(branch)?;
        guard_base(base)?;
        let request = self
            .request(reqwest::Method::POST, repo, "/pulls")
            .await?
            .json(&serde_json::json!({
                "title": title,
                "body": body,
                "head": branch,
                "base": base,
                "draft": true,
            }));
        pull_from(&self.send_json(request).await?)
    }

    async fn update_pull_body(
        &self,
        repo: &str,
        pull_number: i64,
        body: &str,
    ) -> Result<(), GithubError> {
        self.ensure_writable(repo)?;
        let request = self
            .request(
                reqwest::Method::PATCH,
                repo,
                &format!("/pulls/{pull_number}"),
            )
            .await?
            .json(&serde_json::json!({ "body": body }));
        self.send_json(request).await?;
        Ok(())
    }

    async fn comment_on_pull(
        &self,
        repo: &str,
        pull_number: i64,
        body: &str,
    ) -> Result<(), GithubError> {
        self.ensure_writable(repo)?;
        let request = self
            .request(
                reqwest::Method::POST,
                repo,
                &format!("/issues/{pull_number}/comments"),
            )
            .await?
            .json(&serde_json::json!({ "body": body }));
        self.send_json(request).await?;
        Ok(())
    }

    async fn pull_brief(
        &self,
        repo: &str,
        pull_number: i64,
    ) -> Result<GithubPullBrief, GithubError> {
        let value = self
            .send_json(
                self.request(reqwest::Method::GET, repo, &format!("/pulls/{pull_number}"))
                    .await?,
            )
            .await?;
        let number = value
            .get("number")
            .and_then(serde_json::Value::as_i64)
            .filter(|n| *n > 0)
            .ok_or_else(|| GithubError::Http("github pull has no number".into()))?;
        Ok(GithubPullBrief {
            number,
            head: field(&value, "/head/ref")?,
            base: field(&value, "/base/ref")?,
            // Missing means malformed; fail closed by treating it as
            // not-a-draft rather than flipping blind.
            draft: value
                .get("draft")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            // The write path keys on this; a brief without it cannot flip,
            // so it is an error rather than an empty string that would
            // mutate the wrong PR or none at all.
            node_id: value
                .get("node_id")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| GithubError::Http("github pull has no node_id".into()))?,
            same_repo: match (
                value
                    .pointer("/head/repo/full_name")
                    .and_then(serde_json::Value::as_str),
                value
                    .pointer("/base/repo/full_name")
                    .and_then(serde_json::Value::as_str),
            ) {
                (Some(head), Some(base)) => head.eq_ignore_ascii_case(base),
                _ => false,
            },
            open: value.get("state").and_then(serde_json::Value::as_str) == Some("open"),
            head_sha: field(&value, "/head/sha")?,
            labels: label_names(&value),
            merged_at: timestamp(&value, "/merged_at"),
            closed_at: timestamp(&value, "/closed_at"),
            merge_commit_sha: value
                .get("merge_commit_sha")
                .and_then(serde_json::Value::as_str)
                .filter(|sha| maidan_types::is_git_sha(sha))
                .map(str::to_ascii_lowercase),
        })
    }

    async fn pull_snapshot(
        &self,
        repo: &str,
        pull_number: i64,
    ) -> Result<GithubPullSnapshot, GithubError> {
        let (owner, name) = repo
            .split_once('/')
            .ok_or_else(|| GithubError::Refused(format!("`{repo}` is not owner/name")))?;
        let request = self.graphql(repo, false).await?.json(&serde_json::json!({
            "query": PULL_SNAPSHOT_QUERY,
            "variables": { "owner": owner, "name": name, "number": pull_number },
        }));
        let mut snapshot = snapshot_from(&self.send_json(request).await?)?;
        if snapshot.checks_denied {
            // A fine-grained token cannot be granted check runs at all, but
            // it can read Actions and commit statuses: read the head's CI
            // from those. Still refused, the snapshot stays denied and the
            // sweep says what the token lacks.
            let head = snapshot.brief.head_sha.clone();
            // Every run is read down to its jobs, so the required contexts
            // stay in force: one not yet reported holds the gate.
            if let Ok(checks) = self.actions_checks(repo, &head).await {
                snapshot.checks = Some(checks);
                snapshot.checks_denied = false;
            }
        }
        Ok(snapshot)
    }

    async fn commit_on_branch(
        &self,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<bool, GithubError> {
        let branch = safe_ref_name(branch)?;
        if !maidan_types::is_git_sha(sha) {
            return Err(GithubError::Refused(format!("`{sha}` is not a commit")));
        }
        let value = self
            .send_json(
                self.request(
                    reqwest::Method::GET,
                    repo,
                    &format!("/compare/{}...{sha}", encode_path(branch)),
                )
                .await?,
            )
            .await?;
        Ok(matches!(
            value.get("status").and_then(serde_json::Value::as_str),
            Some("behind" | "identical")
        ))
    }

    async fn job_log_tail(
        &self,
        repo: &str,
        job_id: i64,
        max_bytes: usize,
    ) -> Result<String, GithubError> {
        // GitHub answers with a redirect to short-lived log storage, which
        // the client follows; reqwest drops the bearer when the host changes.
        let mut resp = self
            .request(
                reqwest::Method::GET,
                repo,
                &format!("/actions/jobs/{job_id}/logs"),
            )
            .await?
            .send()
            .await
            .map_err(|e| self.http_error(e))?;
        if !resp.status().is_success() {
            return Err(GithubError::Api {
                status: resp.status().as_u16(),
                rate_limited: is_rate_limited(resp.headers()),
            });
        }
        // Streamed, keeping only the end, so a long log never sits in memory
        // whole; the client's timeout bounds how long that takes.
        let mut tail: Vec<u8> = Vec::new();
        let mut cut = false;
        while let Some(chunk) = resp.chunk().await.map_err(|e| self.http_error(e))? {
            tail.extend_from_slice(&chunk);
            if tail.len() > max_bytes {
                tail.drain(..tail.len() - max_bytes);
                cut = true;
            }
        }
        Ok(log_tail_text(&tail, cut))
    }

    async fn branch_commits_since(
        &self,
        repo: &str,
        branch: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<GithubCommitBrief>, GithubError> {
        let branch = safe_ref_name(branch)?;
        let since = since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let value = self
            .send_json(
                self.request(
                    reqwest::Method::GET,
                    repo,
                    &format!("/commits?sha={branch}&since={since}&per_page=100"),
                )
                .await?,
            )
            .await?;
        Ok(list(&value)
            .filter_map(|commit| {
                Some(GithubCommitBrief {
                    sha: commit.get("sha")?.as_str()?.to_ascii_lowercase(),
                    message: commit.pointer("/commit/message")?.as_str()?.to_string(),
                })
            })
            .collect())
    }

    async fn merged_pulls_into(
        &self,
        repo: &str,
        base: &str,
    ) -> Result<Vec<GithubMergedPull>, GithubError> {
        let base = safe_ref_name(base)?;
        let value = self
            .send_json(
                self.request(
                    reqwest::Method::GET,
                    repo,
                    &format!(
                        "/pulls?state=closed&base={base}&sort=updated&direction=desc&per_page=100"
                    ),
                )
                .await?,
            )
            .await?;
        Ok(list(&value)
            .filter_map(|pull| {
                Some(GithubMergedPull {
                    number: pull.get("number")?.as_i64()?,
                    title: pull.get("title")?.as_str()?.to_string(),
                    body: pull
                        .get("body")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    merged_at: timestamp(pull, "/merged_at")?,
                })
            })
            .collect())
    }

    async fn pull_commit_shas(
        &self,
        repo: &str,
        pull_number: i64,
    ) -> Result<Vec<String>, GithubError> {
        let value = self
            .send_json(
                self.request(
                    reqwest::Method::GET,
                    repo,
                    &format!("/pulls/{pull_number}/commits?per_page=100"),
                )
                .await?,
            )
            .await?;
        Ok(list(&value)
            .filter_map(|commit| commit.get("sha")?.as_str().map(str::to_ascii_lowercase))
            .collect())
    }

    async fn pull_review_activity(
        &self,
        repo: &str,
        pull_number: i64,
    ) -> Result<Vec<GithubReviewActivity>, GithubError> {
        let mut activity = Vec::new();
        for (path, at) in [("reviews", "/submitted_at"), ("comments", "/created_at")] {
            let value = self
                .send_json(
                    self.request(
                        reqwest::Method::GET,
                        repo,
                        &format!("/pulls/{pull_number}/{path}?per_page=100"),
                    )
                    .await?,
                )
                .await?;
            activity.extend(
                list(&value)
                    // A pending review is a draft nobody else can see yet.
                    .filter(|item| {
                        item.get("state").and_then(serde_json::Value::as_str) != Some("PENDING")
                    })
                    .map(|item| GithubReviewActivity {
                        bot: item
                            .pointer("/user/type")
                            .and_then(serde_json::Value::as_str)
                            == Some("Bot")
                            || item
                                .pointer("/user/login")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|login| login.ends_with("[bot]")),
                        at: timestamp(item, at),
                    }),
            );
        }
        Ok(activity)
    }

    async fn commits_behind(
        &self,
        repo: &str,
        base_sha: &str,
        branch: &str,
    ) -> Result<i64, GithubError> {
        // A read, so no branch policy applies; the name only has to be safe
        // in the URL.
        let safe = !branch.is_empty()
            && !branch.contains("..")
            && branch
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
        if !safe || !maidan_types::is_git_sha(base_sha) {
            return Err(GithubError::Refused(format!(
                "cannot compare `{base_sha}` with `{branch}`"
            )));
        }
        let value = self
            .send_json(
                self.request(
                    reqwest::Method::GET,
                    repo,
                    &format!("/compare/{base_sha}...{branch}"),
                )
                .await?,
            )
            .await?;
        value
            .get("ahead_by")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| GithubError::Http("github compare has no ahead_by".into()))
    }

    async fn add_pull_label(
        &self,
        repo: &str,
        pull_number: i64,
        label: &str,
        _pass: crate::routes::EnqueueGuardPass,
    ) -> Result<(), GithubError> {
        self.ensure_writable(repo)?;
        let request = self
            .request(
                reqwest::Method::POST,
                repo,
                &format!("/issues/{pull_number}/labels"),
            )
            .await?
            .json(&serde_json::json!({ "labels": [label] }));
        let value = self.send_json(request).await?;
        // The answer is the issue's labels after the write; the label counts
        // as applied only when it is among them.
        let applied = value.as_array().is_some_and(|labels| {
            labels
                .iter()
                .any(|l| l.get("name").and_then(serde_json::Value::as_str) == Some(label))
        });
        if applied {
            Ok(())
        } else {
            Err(GithubError::Http(format!(
                "github label unverified: `{label}` not on {repo}#{pull_number} after the write"
            )))
        }
    }

    async fn remove_pull_label(
        &self,
        repo: &str,
        pull_number: i64,
        label: &str,
        _pass: crate::routes::EnqueueGuardPass,
    ) -> Result<bool, GithubError> {
        self.ensure_writable(repo)?;
        let request = self
            .request(
                reqwest::Method::DELETE,
                repo,
                &format!(
                    "/issues/{pull_number}/labels/{}",
                    urlencoding::encode(label)
                ),
            )
            .await?;
        match self.send_json(request).await {
            Ok(_) => Ok(true),
            // GitHub answers 404 when the label is not on the issue: already
            // dequeued, which is the state the caller asked for.
            Err(GithubError::Api { status: 404, .. }) => Ok(false),
            Err(err) => Err(err),
        }
    }

    async fn set_pull_ready(
        &self,
        repo: &str,
        pull_number: i64,
        node_id: &str,
        _pass: MarkReadyGuardPass,
    ) -> Result<(), GithubError> {
        self.ensure_writable(repo)?;
        // REST `PATCH {"draft": false}` is a silent no-op on GitHub — 200,
        // draft unchanged — so the flip goes through the GraphQL
        // `markPullRequestReadyForReview` mutation, on the same token.
        let request = self
            .graphql(repo, true).await?
            .json(&serde_json::json!({
                "query": "mutation($nodeId: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $nodeId}) { pullRequest { isDraft } } }",
                "variables": { "nodeId": node_id },
            }));
        let value = self.send_json(request).await?;
        // GraphQL answers 200 with an `errors` array when the mutation is
        // rejected — an already-ready PR, a revoked token — so a missing
        // `data` is never success.
        if let Some(errors) = value
            .get("errors")
            .and_then(|e| e.as_array())
            .filter(|e| !e.is_empty())
        {
            return Err(GithubError::Http(format!(
                "github markPullRequestReadyForReview rejected: {errors:?}"
            )));
        }
        // The flip counts only when the mutation's own answer says the PR is
        // no longer a draft; anything else is a failed write, not a success.
        match value
            .pointer("/data/markPullRequestReadyForReview/pullRequest/isDraft")
            .and_then(serde_json::Value::as_bool)
        {
            Some(false) => Ok(()),
            other => Err(GithubError::Http(format!(
                "github flip unverified: isDraft={other:?} for {repo}#{pull_number}"
            ))),
        }
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
pub(crate) fn is_rate_limited(headers: &reqwest::header::HeaderMap) -> bool {
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

/// The items of a GitHub list answer; none when it is not a list.
fn list(value: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    value.as_array().into_iter().flatten()
}

/// A branch name safe to put in a query string unescaped, or a refusal. A
/// read, so no branch policy applies.
fn safe_ref_name(name: &str) -> Result<&str, GithubError> {
    let safe = !name.is_empty()
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    if safe {
        Ok(name)
    } else {
        Err(GithubError::Refused(format!(
            "`{name}` is not a branch name"
        )))
    }
}

/// The change sweep's per-tick read of one pull request: the brief the
/// sweep settles closes and readiness from, and the checks on the head
/// commit.
pub(crate) const PULL_SNAPSHOT_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { \
repository(owner: $owner, name: $name) { pullRequest(number: $number) { \
id number state isDraft isCrossRepository headRefName headRefOid baseRefName \
mergedAt closedAt mergeCommit { oid } labels(first: 50) { nodes { name } } \
ahead: commits { totalCount } \
mergeStateStatus \
reviewThreads(first: 100) { nodes { isResolved comments(first: 1) { nodes { url author { login } } } } } \
baseRef { branchProtectionRule { requiredStatusCheckContexts } } \
mergeQueueEntry { state } \
timelineItems(last: 1, itemTypes: [ADDED_TO_MERGE_QUEUE_EVENT, REMOVED_FROM_MERGE_QUEUE_EVENT]) { nodes { \
__typename ... on RemovedFromMergeQueueEvent { reason } } } \
commits(last: 1) { nodes { commit { oid statusCheckRollup { contexts(first: 100) { nodes { \
__typename \
... on CheckRun { name status conclusion databaseId checkSuite { app { slug } } } \
... on StatusContext { context state } \
} } } } } } } } }";

/// A [`PULL_SNAPSHOT_QUERY`] answer as a snapshot. GraphQL answers a missing
/// pull request with `200` and an `errors` array, so its error types are
/// mapped back onto the statuses REST would have sent: `NOT_FOUND` is a 404
/// and `RATE_LIMITED` a rate-limited 403. Only the types are kept, never a
/// message.
pub(crate) fn snapshot_from(value: &serde_json::Value) -> Result<GithubPullSnapshot, GithubError> {
    let pull = value
        .pointer("/data/repository/pullRequest")
        .filter(|pull| pull.is_object());
    let Some(pull) = pull else {
        let types: Vec<&str> = value
            .get("errors")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| e.get("type").and_then(serde_json::Value::as_str))
            .collect();
        return Err(if types.contains(&"RATE_LIMITED") {
            GithubError::Api {
                status: 403,
                rate_limited: true,
            }
        } else if types.contains(&"NOT_FOUND") || value.pointer("/data/repository").is_some() {
            GithubError::Api {
                status: 404,
                rate_limited: false,
            }
        } else {
            GithubError::Http(format!(
                "github graphql answered no pull request: {types:?}"
            ))
        });
    };
    let number = pull
        .get("number")
        .and_then(serde_json::Value::as_i64)
        .filter(|n| *n > 0)
        .ok_or_else(|| GithubError::Http("github pull has no number".into()))?;
    let state = pull.get("state").and_then(serde_json::Value::as_str);
    let brief = GithubPullBrief {
        number,
        head: field(pull, "/headRefName")?,
        base: field(pull, "/baseRefName")?,
        draft: pull
            .get("isDraft")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        node_id: field(pull, "/id")?,
        // A head GitHub cannot place reads as a fork, as the REST brief's
        // missing head repository does.
        same_repo: pull
            .get("isCrossRepository")
            .and_then(serde_json::Value::as_bool)
            == Some(false),
        open: state == Some("OPEN"),
        head_sha: field(pull, "/headRefOid")?.to_ascii_lowercase(),
        labels: pull
            .pointer("/labels/nodes")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|label| label.get("name").and_then(serde_json::Value::as_str))
            .map(str::to_string)
            .collect(),
        merged_at: timestamp(pull, "/mergedAt"),
        closed_at: timestamp(pull, "/closedAt"),
        merge_commit_sha: pull
            .pointer("/mergeCommit/oid")
            .and_then(serde_json::Value::as_str)
            .filter(|sha| maidan_types::is_git_sha(sha))
            .map(str::to_ascii_lowercase),
    };
    let commit = pull.pointer("/commits/nodes/0/commit");
    // Checks only for the head the brief names: a push between GitHub's two
    // reads would otherwise pin one head's checks on another.
    let checks = commit
        .filter(|commit| {
            commit
                .get("oid")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|oid| oid.eq_ignore_ascii_case(&brief.head_sha))
        })
        .map(|commit| {
            commit
                .pointer("/statusCheckRollup/contexts/nodes")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(check_from)
                .collect()
        });
    let text_list = |pointer: &str| {
        pull.pointer(pointer)
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
    };
    let gate = GithubGate {
        merge_state: pull
            .get("mergeStateStatus")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        unresolved_threads: pull
            .pointer("/reviewThreads/nodes")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|thread| {
                thread
                    .get("isResolved")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
            })
            .map(|thread| maidan_types::ReviewThreadLink {
                url: thread
                    .pointer("/comments/nodes/0/url")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                author: thread
                    .pointer("/comments/nodes/0/author/login")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            })
            .collect(),
        required_checks: text_list("/baseRef/branchProtectionRule/requiredStatusCheckContexts"),
        queued: pull
            .get("mergeQueueEntry")
            .is_some_and(serde_json::Value::is_object),
        dequeued: pull
            .pointer("/timelineItems/nodes/0")
            .filter(|event| {
                event.get("__typename").and_then(serde_json::Value::as_str)
                    == Some("RemovedFromMergeQueueEvent")
            })
            .map(|event| {
                event
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("UNKNOWN")
                    .to_string()
            }),
    };
    Ok(GithubPullSnapshot {
        brief,
        checks,
        checks_denied: rollup_denied(value),
        gate: Some(gate),
        // A pull request lists the commits its head has that its base lacks.
        ahead: pull
            .pointer("/ahead/totalCount")
            .and_then(serde_json::Value::as_i64)
            .map(|commits| commits > 0),
    })
}

/// A REST run or job's state, in the GraphQL spelling [`check_from`] reads.
fn upper(node: &serde_json::Value, key: &str) -> serde_json::Value {
    node.get(key)
        .and_then(serde_json::Value::as_str)
        .map_or(serde_json::Value::Null, |text| {
            serde_json::Value::String(text.to_ascii_uppercase())
        })
}

/// The most pages of 100 the Actions fallback reads from one list; a head
/// with more runs, jobs or statuses than that stays pending.
const ACTIONS_MAX_PAGES: u32 = 5;

/// A workflow run as a rollup `CheckRun` node, named for its workflow.
fn run_node(run: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "__typename": "CheckRun",
        "name": run.get("name").cloned().unwrap_or(serde_json::Value::Null),
        "status": upper(run, "status"),
        "conclusion": upper(run, "conclusion"),
    })
}

/// The checks the Actions and commit-status answers describe: one per job
/// of each run, then one per commit status. A run with no jobs read, or
/// whose own failed or pending state no job carries (a startup failure, a
/// run still creating its jobs), is also read whole under its workflow's
/// name, so its state is never lost. Read through [`check_from`], so a state
/// means here what it means in the rollup.
fn actions_checks_from(
    runs: &[serde_json::Value],
    jobs: &std::collections::HashMap<i64, Vec<serde_json::Value>>,
    statuses: &[serde_json::Value],
) -> Vec<GithubCheck> {
    let mut checks = Vec::new();
    for run in runs {
        let run_jobs: Vec<GithubCheck> = run
            .get("id")
            .and_then(serde_json::Value::as_i64)
            .and_then(|id| jobs.get(&id))
            .map(|answer| {
                answer
                    .iter()
                    .filter_map(|job| {
                        check_from(&serde_json::json!({
                            "__typename": "CheckRun",
                            "name": job.get("name").cloned().unwrap_or(serde_json::Value::Null),
                            "status": upper(job, "status"),
                            "conclusion": upper(job, "conclusion"),
                            "databaseId": job.get("id").cloned().unwrap_or(serde_json::Value::Null),
                            "checkSuite": {"app": {"slug": "github-actions"}},
                        }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let whole = check_from(&run_node(run));
        let carried = whole.as_ref().is_none_or(|whole| {
            whole.state == GithubCheckState::Passed
                || run_jobs.iter().any(|job| job.state == whole.state)
        });
        let unopened = run_jobs.is_empty();
        checks.extend(run_jobs);
        if unopened || !carried {
            checks.extend(whole);
        }
    }
    checks.extend(statuses.iter().filter_map(|status| {
        check_from(&serde_json::json!({
            "__typename": "StatusContext",
            "context": status.get("context").cloned().unwrap_or(serde_json::Value::Null),
            "state": upper(status, "state"),
        }))
    }));
    checks
}

/// Whether GitHub refused the head's `statusCheckRollup`. A fine-grained
/// token without Checks and Commit statuses read reads the pull request
/// fine, and GitHub answers the rollup `null` with a `FORBIDDEN` error beside
/// the data rather than failing the read.
fn rollup_denied(value: &serde_json::Value) -> bool {
    value
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|error| {
            error.get("type").and_then(serde_json::Value::as_str) == Some("FORBIDDEN")
                && error
                    .get("path")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|path| {
                        path.iter()
                            .any(|part| part.as_str() == Some("statusCheckRollup"))
                    })
        })
}

/// One `statusCheckRollup` context as a check, or `None` for a node this
/// client does not know.
fn check_from(node: &serde_json::Value) -> Option<GithubCheck> {
    let text = |key: &str| node.get(key).and_then(serde_json::Value::as_str);
    match text("__typename")? {
        "CheckRun" => {
            let state = if text("status") != Some("COMPLETED") {
                GithubCheckState::Pending
            } else {
                match text("conclusion") {
                    Some(
                        "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
                        | "STARTUP_FAILURE",
                    ) => GithubCheckState::Failed,
                    _ => GithubCheckState::Passed,
                }
            };
            let actions = node
                .pointer("/checkSuite/app/slug")
                .and_then(serde_json::Value::as_str)
                == Some("github-actions");
            Some(GithubCheck {
                name: text("name")?.to_string(),
                state,
                job_id: node
                    .get("databaseId")
                    .and_then(serde_json::Value::as_i64)
                    .filter(|_| actions),
            })
        }
        "StatusContext" => Some(GithubCheck {
            name: text("context")?.to_string(),
            state: match text("state") {
                Some("SUCCESS") => GithubCheckState::Passed,
                Some("FAILURE" | "ERROR") => GithubCheckState::Failed,
                _ => GithubCheckState::Pending,
            },
            job_id: None,
        }),
        _ => None,
    }
}

/// The end of a log as text. When the start was cut away, the partial first
/// line goes too, so the tail starts at a line.
pub(crate) fn log_tail_text(tail: &[u8], cut: bool) -> String {
    let text = String::from_utf8_lossy(tail);
    if cut {
        text.split_once('\n')
            .map_or_else(String::new, |(_, rest)| rest.to_string())
    } else {
        text.into_owned()
    }
}

/// An RFC 3339 timestamp at `pointer`, or `None` when absent or null.
fn timestamp(value: &serde_json::Value, pointer: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&chrono::Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every request the client sends is made in `build`, and only the two
    /// builders that check `MAIDAN_GITHUB_WRITE_REPOS` call it, so a write
    /// method added later cannot reach GitHub without the check. The one
    /// other use is `bearer` handing the client to the GitHub App's token
    /// exchange, which writes to no repository.
    #[test]
    fn every_github_request_is_built_where_the_repository_is_checked() {
        let source = include_str!("github.rs");
        let source = &source[..source
            .find("#[cfg(test)]\nmod tests")
            .unwrap_or(source.len())];
        let http_uses = source
            .match_indices("self.http")
            .filter(|(at, found)| {
                !source[at + found.len()..].starts_with(|c: char| c == '_' || c.is_alphanumeric())
            })
            .count();
        assert_eq!(
            http_uses, 2,
            "only `build` and `bearer` (the GitHub App token exchange) may use self.http"
        );
        assert_eq!(
            source.matches("self.build(").count(),
            2,
            "only `request_accepting` and `graphql` may call `build`"
        );
    }

    #[test]
    fn from_env_disabled_without_a_secret() {
        // Not asserting on process env here (shared) — just the shape: a config with
        // an empty/unset secret must not enable the projector. Verified via the
        // ingress e2e (404 when unconfigured).
        let cfg = GithubConfig {
            webhook_secret: "s".into(),
            api_token: None,
            write_repos: Vec::new(),
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
            write_repos: Vec::new(),
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

    #[test]
    fn the_app_installation_token_never_reaches_an_error() {
        let app = std::sync::Arc::new(
            crate::github_app::GithubAppAuth::new(
                "1",
                "2",
                &crate::github_app::github_app_key::test_key().pkcs1_pem,
            )
            .unwrap(),
        );
        app.seed_token_for_test("ghs_installationsecret");
        let client = GithubApiClient::with_app(app);
        let err = client.http_error("request to https://x/?t=ghs_installationsecret failed");
        assert!(!err.to_string().contains("ghs_installationsecret"), "{err}");
    }

    #[tokio::test]
    async fn the_client_writes_only_agent_branches_and_never_opens_a_pull_into_prod() {
        // No server: a refused write must not reach the network at all.
        let client = GithubApiClient::with_base_url("t".into(), "http://127.0.0.1:9".into())
            .with_any_write_repo();
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

    fn snapshot_answer(
        head: &str,
        commit_oid: &str,
        contexts: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({"data": {"repository": {"pullRequest": {
            "id": "PR_7", "number": 7, "state": "OPEN", "isDraft": false,
            "isCrossRepository": false, "headRefName": "feature/x", "headRefOid": head,
            "baseRefName": "main", "mergedAt": null, "closedAt": null, "mergeCommit": null,
            "labels": {"nodes": [{"name": "ready-to-merge"}]},
            "mergeStateStatus": "BEHIND",
            "reviewThreads": {"nodes": [
                {"isResolved": false, "comments": {"nodes": [
                    {"url": "https://github.com/o/r/pull/7#discussion_r1", "author": {"login": "review-bot[bot]"}}]}},
                {"isResolved": true, "comments": {"nodes": [
                    {"url": "https://github.com/o/r/pull/7#discussion_r2", "author": {"login": "someone"}}]}},
            ]},
            "baseRef": {"branchProtectionRule": {"requiredStatusCheckContexts": ["lint", "e2e"]}},
            "mergeQueueEntry": null,
            "timelineItems": {"nodes": [{"__typename": "RemovedFromMergeQueueEvent", "reason": "MERGE_CONFLICT"}]},
            "commits": {"nodes": [{"commit": {"oid": commit_oid,
                "statusCheckRollup": {"contexts": {"nodes": contexts}}}}]},
        }}}})
    }

    #[test]
    fn a_snapshot_reads_the_brief_and_classifies_each_check() {
        let snapshot = snapshot_from(&snapshot_answer(
            "ABC",
            "abc",
            serde_json::json!([
                {"__typename": "CheckRun", "name": "lint", "status": "COMPLETED",
                 "conclusion": "CANCELLED", "databaseId": 5, "checkSuite": {"app": {"slug": "github-actions"}}},
                {"__typename": "CheckRun", "name": "review", "status": "QUEUED",
                 "conclusion": null, "databaseId": 6, "checkSuite": {"app": {"slug": "other-app"}}},
                {"__typename": "StatusContext", "context": "ci/legacy", "state": "ERROR"},
                {"__typename": "StatusContext", "context": "ci/new", "state": "EXPECTED"},
                {"__typename": "Unknown"},
            ]),
        ))
        .unwrap();
        assert!(snapshot.brief.open && snapshot.brief.same_repo);
        assert_eq!(snapshot.brief.head_sha, "abc");
        assert_eq!(snapshot.brief.labels, vec!["ready-to-merge".to_string()]);
        let gate = snapshot.gate.clone().unwrap();
        assert_eq!(gate.merge_state.as_deref(), Some("BEHIND"));
        assert!(!gate.queued);
        assert_eq!(gate.dequeued.as_deref(), Some("MERGE_CONFLICT"));
        assert_eq!(
            gate.required_checks,
            Some(vec!["lint".to_string(), "e2e".to_string()])
        );
        assert_eq!(
            gate.unresolved_threads,
            vec![maidan_types::ReviewThreadLink {
                url: "https://github.com/o/r/pull/7#discussion_r1".into(),
                author: Some("review-bot[bot]".into()),
            }]
        );
        let checks = snapshot.checks.unwrap();
        let read: Vec<(&str, GithubCheckState, Option<i64>)> = checks
            .iter()
            .map(|c| (c.name.as_str(), c.state, c.job_id))
            .collect();
        assert_eq!(
            read,
            vec![
                ("lint", GithubCheckState::Failed, Some(5)),
                ("review", GithubCheckState::Pending, None),
                ("ci/legacy", GithubCheckState::Failed, None),
                ("ci/new", GithubCheckState::Pending, None),
            ]
        );
    }

    #[test]
    fn checks_on_another_commit_than_the_head_are_not_read() {
        let snapshot =
            snapshot_from(&snapshot_answer("abc", "def", serde_json::json!([]))).unwrap();
        assert_eq!(snapshot.checks, None);
    }

    #[test]
    fn a_rollup_github_refuses_the_token_reads_as_denied_not_as_no_checks() {
        let mut denied = snapshot_answer("abc", "abc", serde_json::json!([]));
        denied["data"]["repository"]["pullRequest"]["commits"]["nodes"][0]["commit"]
            ["statusCheckRollup"] = serde_json::Value::Null;
        denied["errors"] = serde_json::json!([{
            "type": "FORBIDDEN",
            "path": ["repository", "pullRequest", "commits", "nodes", 0, "commit", "statusCheckRollup"],
            "message": "Resource not accessible by personal access token",
        }]);
        let snapshot = snapshot_from(&denied).unwrap();
        assert!(snapshot.checks_denied);
        assert_eq!(snapshot.checks, Some(vec![]));

        let readable =
            snapshot_from(&snapshot_answer("abc", "abc", serde_json::json!([]))).unwrap();
        assert!(!readable.checks_denied);

        // A refusal elsewhere in the answer is not the checks'.
        let mut other = snapshot_answer("abc", "abc", serde_json::json!([]));
        other["errors"] = serde_json::json!([{
            "type": "FORBIDDEN",
            "path": ["repository", "pullRequest", "baseRef", "branchProtectionRule"],
        }]);
        assert!(!snapshot_from(&other).unwrap().checks_denied);
    }

    #[test]
    fn the_actions_fallback_reads_every_run_to_its_jobs_and_adds_statuses() {
        let runs = vec![
            serde_json::json!({"id": 1, "name": "CI", "status": "completed", "conclusion": "failure"}),
            serde_json::json!({"id": 2, "name": "Docs", "status": "completed", "conclusion": "success"}),
            serde_json::json!({"id": 3, "name": "E2E", "status": "in_progress", "conclusion": null}),
            serde_json::json!({"id": 4, "name": "Lint", "status": "completed", "conclusion": "startup_failure"}),
        ];
        let job = |id: i64, name: &str, status: &str, conclusion: Option<&str>| serde_json::json!({"id": id, "name": name, "status": status, "conclusion": conclusion});
        let jobs = std::collections::HashMap::from([
            (
                1,
                vec![
                    job(11, "unit tests", "completed", Some("failure")),
                    job(12, "clippy", "completed", Some("success")),
                ],
            ),
            (2, vec![job(21, "mdbook", "completed", Some("success"))]),
            // E2E has created no jobs yet, and Lint failed before running any
            // job: each is read whole, so neither reads as passed.
            (3, vec![]),
            (4, vec![job(41, "fmt", "completed", Some("skipped"))]),
        ]);
        let statuses = vec![
            serde_json::json!({"context": "ci/legacy", "state": "error"}),
            serde_json::json!({"context": "deploy/preview", "state": "pending"}),
        ];
        let read: Vec<(String, GithubCheckState, Option<i64>)> =
            actions_checks_from(&runs, &jobs, &statuses)
                .into_iter()
                .map(|c| (c.name, c.state, c.job_id))
                .collect();
        assert_eq!(
            read,
            vec![
                ("unit tests".into(), GithubCheckState::Failed, Some(11)),
                ("clippy".into(), GithubCheckState::Passed, Some(12)),
                ("mdbook".into(), GithubCheckState::Passed, Some(21)),
                ("E2E".into(), GithubCheckState::Pending, None),
                ("fmt".into(), GithubCheckState::Passed, Some(41)),
                ("Lint".into(), GithubCheckState::Failed, None),
                ("ci/legacy".into(), GithubCheckState::Failed, None),
                ("deploy/preview".into(), GithubCheckState::Pending, None),
            ]
        );
        let summary = crate::ci_watch::summarize(&actions_checks_from(&runs, &jobs, &statuses));
        assert_eq!(
            summary.first_failing_job,
            Some(11),
            "the failed job's log is the one fetched"
        );
    }

    #[test]
    fn a_required_job_that_has_not_reported_holds_the_fallback_gate() {
        // Only Docs has run: its job passes, but the required `unit tests`
        // context never reported, which the gate must not read as clear.
        let runs = vec![serde_json::json!(
            {"id": 2, "name": "Docs", "status": "completed", "conclusion": "success"}
        )];
        let jobs = std::collections::HashMap::from([(
            2,
            vec![serde_json::json!(
                {"id": 21, "name": "mdbook", "status": "completed", "conclusion": "success"}
            )],
        )]);
        let checks = actions_checks_from(&runs, &jobs, &[]);
        let read = GithubGate {
            required_checks: Some(vec!["unit tests".into()]),
            ..GithubGate::default()
        };
        assert_eq!(
            crate::ci_watch::gate(&read, &checks).missing_checks,
            vec!["unit tests".to_string()]
        );
    }

    #[test]
    fn a_graphql_miss_or_rate_limit_maps_onto_the_rest_status() {
        let missing = serde_json::json!({
            "data": {"repository": {"pullRequest": null}},
            "errors": [{"type": "NOT_FOUND", "message": "no such pull"}],
        });
        assert!(snapshot_from(&missing).unwrap_err().is_not_found());
        let limited = serde_json::json!({"errors": [{"type": "RATE_LIMITED"}]});
        let err = snapshot_from(&limited).unwrap_err();
        assert!(matches!(
            err,
            GithubError::Api {
                status: 403,
                rate_limited: true
            }
        ));
        assert!(!err.is_misconfiguration());
    }

    #[test]
    fn a_cut_log_tail_starts_at_a_line() {
        assert_eq!(
            log_tail_text(b"tial line\nnext\nlast\n", true),
            "next\nlast\n"
        );
        assert_eq!(log_tail_text(b"whole\n", false), "whole\n");
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
