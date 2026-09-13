//! Slack projector — ingress foundation (Cluster 307).
//!
//! A *projector*, not a bot: it relays between a Slack channel and a Maidan
//! channel with **no LLM in Maidan** (Expansion Bets, Bet 1). This cluster lands
//! the ingress foundation — request-signature verification and the Slack Events
//! API `url_verification` handshake — so a Slack app can be pointed at
//! `POST /integrations/slack/events`. Channel-link mapping + message → thread
//! posting is Cluster 308; egress (Maidan → Slack) is 309.
//!
//! **Config-gated:** inert unless `MAIDAN_SLACK_SIGNING_SECRET` is set (the route
//! then returns `404`), so an unconfigured deployment is unchanged.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use hmac::{Hmac, Mac};
use maidan_auth::{capability::WORKSPACE_READ, capability::WORKSPACE_WRITE, AuthContext};
use maidan_types::{
    EgressKind, EgressTarget, ExternalRef, MemberId, NewEgressOutbox, NewSlackChannelLink,
    SlackChannelLink, ThreadId, WorkspaceId,
};
use sha2::Sha256;

use crate::dto::LinkSlackChannel;
use crate::error::ApiJson;
use crate::routes::{cap, ensure_workspace, ApiResult};
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

/// Reject a request whose `X-Slack-Request-Timestamp` is more than this far from
/// now (Slack's replay-protection recommendation).
const SLACK_MAX_SKEW_SECS: i64 = 300;

/// Slack app credentials. `signing_secret` verifies inbound requests; `bot_token`
/// (optional here) authorizes outbound Web API calls in the egress cluster (309).
#[derive(Debug, Clone)]
pub struct SlackConfig {
    pub signing_secret: String,
    pub bot_token: Option<String>,
}

impl SlackConfig {
    /// Build from the environment, or `None` when `MAIDAN_SLACK_SIGNING_SECRET` is
    /// unset — the projector is then disabled.
    pub fn from_env() -> Option<SlackConfig> {
        let signing_secret = std::env::var("MAIDAN_SLACK_SIGNING_SECRET")
            .ok()
            .filter(|s| !s.is_empty())?;
        let bot_token = std::env::var("MAIDAN_SLACK_BOT_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        Some(SlackConfig {
            signing_secret,
            bot_token,
        })
    }
}

/// Compute the `X-Slack-Signature` a Slack sender produces for a request: Slack
/// signs `v0:{timestamp}:{body}` with the app signing secret and formats it
/// `v0={hex}`. The inverse of [`verify_slack_signature`] — useful for tests / mock
/// senders (Maidan is the receiver, so it only verifies in prod).
pub fn slack_signature(signing_secret: &str, timestamp: &str, body: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .unwrap_or_else(|_| unreachable!("HMAC-SHA256 accepts any key length"));
    mac.update(format!("v0:{timestamp}:{body}").as_bytes());
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
}

/// Verify a Slack request signature (see [`slack_signature`]). The timestamp must
/// be within ±5 min (replay protection); comparison is constant-time.
pub fn verify_slack_signature(
    signing_secret: &str,
    timestamp: &str,
    body: &str,
    signature: &str,
    now_unix: i64,
) -> bool {
    let ts: i64 = match timestamp.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if (now_unix - ts).abs() > SLACK_MAX_SKEW_SECS {
        return false;
    }
    let expected = slack_signature(signing_secret, timestamp, body);
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), signature.as_bytes()).into()
}

/// `POST /integrations/slack/events` — the Slack Events API ingress. Returns `404`
/// when the projector is not configured, `401` on a bad signature, echoes the
/// `url_verification` challenge during app setup, and ACKs `event_callback`s
/// (message routing to a Maidan thread lands in Cluster 308).
pub async fn slack_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(cfg) = state.slack.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let timestamp = headers
        .get("x-slack-request-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let signature = headers
        .get("x-slack-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let now = chrono::Utc::now().timestamp();
    if !verify_slack_signature(&cfg.signing_secret, timestamp, &body, signature, now) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let payload: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match payload.get("type").and_then(|v| v.as_str()) {
        // App setup: echo the challenge so Slack accepts the events URL.
        Some("url_verification") => {
            let challenge = payload
                .get("challenge")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Json(serde_json::json!({ "challenge": challenge })).into_response()
        }
        // A real event — route it, then ACK fast (Slack retries on non-200).
        Some("event_callback") => {
            if let Some(event) = payload.get("event") {
                route_slack_event(&state, event).await;
            }
            StatusCode::OK.into_response()
        }
        _ => StatusCode::OK.into_response(),
    }
}

/// Route an inbound Slack event: a plain user `message` in a linked channel is
/// posted into the mapped Maidan thread (Cluster 308). Best-effort — the ingress
/// always ACKs. Bot messages and subtype events (edits/deletes/joins) are skipped:
/// only plain user messages project, and skipping `bot_id` avoids echoing our own
/// egress (Cluster 309) back into Maidan.
async fn route_slack_event(state: &AppState, event: &serde_json::Value) {
    if event.get("type").and_then(|v| v.as_str()) != Some("message") {
        return;
    }
    if event.get("bot_id").is_some() || event.get("subtype").is_some() {
        return;
    }
    let (Some(slack_channel), Some(text)) = (
        event.get("channel").and_then(|v| v.as_str()),
        event.get("text").and_then(|v| v.as_str()),
    ) else {
        return;
    };
    let user = event
        .get("user")
        .and_then(|v| v.as_str())
        .unwrap_or("slack");
    let link = match state.store.get_slack_channel_link(slack_channel).await {
        Ok(Some(l)) => l,
        Ok(None) => return, // channel not linked — ignore
        Err(err) => {
            tracing::warn!(error = %err, "slack ingress: link lookup failed");
            return;
        }
    };
    let new = maidan_types::NewMessage {
        thread_id: link.thread_id,
        author_id: link.member_id,
        body: format!("{user}: {text}"),
        // Tag the origin so egress (Cluster 309) never echoes a Slack-sourced
        // message back to Slack (loop prevention).
        metadata: serde_json::json!({ "slack": { "user": user, "channel": slack_channel } }),
        content: None,
    };
    match state.store.post_message_with_event(new, None).await {
        Ok((_, stored)) => crate::routes::publish_stored(state, stored).await,
        Err(err) => tracing::warn!(error = %err, "slack ingress: post failed"),
    }
}

/// A failed Slack Web API call.
#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    #[error("slack http error: {0}")]
    Http(String),
    #[error("slack api error: {0}")]
    Api(String),
}

/// Slack's config-class `chat.postMessage` errors — the analogue of GitHub's
/// 401/403/404 (Cluster 377.3). Slack answers logically, not by status code, so
/// the discriminator is the error string. Notably **absent**: `ratelimited`,
/// `fatal_error` and `service_unavailable`, which are transient and must retry.
const SLACK_MISCONFIGURATION_ERRORS: &[&str] = &[
    // Credentials.
    "invalid_auth",
    "not_authed",
    "account_inactive",
    "token_revoked",
    "token_expired",
    "missing_scope",
    "not_allowed_token_type",
    "no_permission",
    // Destination.
    "channel_not_found",
    "not_in_channel",
    "is_archived",
    "restricted_action",
];

impl SlackError {
    /// Whether this failure is a misconfiguration rather than a transient fault
    /// (Cluster 377.3) — a revoked token, a missing scope, a channel the bot was
    /// removed from or that no longer exists. Retrying cannot fix any of them.
    pub fn is_misconfiguration(&self) -> bool {
        match self {
            Self::Http(_) => false,
            Self::Api(code) => SLACK_MISCONFIGURATION_ERRORS.contains(&code.as_str()),
        }
    }

    /// The message we wanted to edit is gone. Result delivery falls back to a
    /// new post (there is no Slack equivalent of the GitHub body marker).
    pub fn is_message_gone(&self) -> bool {
        matches!(self, Self::Api(code) if code == "message_not_found")
    }
}

/// Outbound Slack sender — `chat.postMessage` / `chat.update` in production, a
/// mock in tests.
#[async_trait::async_trait]
pub trait SlackSender: Send + Sync {
    /// Post a message, returning a handle on it so a later delivery can edit it
    /// in place (Cluster 378.2). `thread_ts` replies inside an existing Slack
    /// thread instead of posting top-level.
    ///
    /// **`Ok(None)` means "posted, but we cannot address it."** Slack answered
    /// `ok: true` without a usable `ts`. That is not a failure — the message
    /// exists — and reporting it as one would make the worker retry and post a
    /// second copy. The caller simply has no ref to store.
    async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError>;

    /// Edit a message posted earlier, addressed by channel + `ts`.
    async fn update_message(&self, channel: &str, ts: &str, text: &str) -> Result<(), SlackError>;
}

/// The production [`SlackSender`]: posts via the Slack Web API `chat.postMessage`.
pub struct SlackWebClient {
    bot_token: String,
    /// API base, `https://slack.com` in production; overridable so the wire path
    /// can be tested against a loopback server (Cluster 347).
    base_url: String,
    http: reqwest::Client,
}

impl SlackWebClient {
    pub fn new(bot_token: String) -> Self {
        Self::with_base_url(bot_token, "https://slack.com".to_string())
    }

    /// Build against a custom API base (test loopback server). `base_url` has no
    /// trailing slash; `/api/chat.postMessage` is appended.
    pub fn with_base_url(bot_token: String, base_url: String) -> Self {
        Self {
            bot_token,
            base_url,
            http: reqwest::Client::new(),
        }
    }

    /// Call a Slack Web API method and decode its envelope. Slack answers logical
    /// errors with HTTP 200 and `{"ok": false, "error": ...}`, so the status is not
    /// the signal — the body is.
    async fn call(
        &self,
        method: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, SlackError> {
        let resp = self
            .http
            .post(format!("{}/api/{method}", self.base_url))
            .bearer_auth(&self.bot_token)
            .json(&payload)
            .send()
            .await
            .map_err(|e| SlackError::Http(e.to_string()))?;
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SlackError::Http(e.to_string()))?;
        if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
            Ok(v)
        } else {
            Err(SlackError::Api(
                v.get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
            ))
        }
    }
}

#[async_trait::async_trait]
impl SlackSender for SlackWebClient {
    async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError> {
        let mut payload = serde_json::json!({ "channel": channel, "text": text });
        // Omitted rather than sent as null: Slack treats an explicit null
        // `thread_ts` as an error, not as "top-level".
        if let Some(parent) = thread_ts {
            payload["thread_ts"] = serde_json::Value::String(parent.to_string());
        }
        let v = self.call("chat.postMessage", payload).await?;
        // The message is posted either way — a missing `ts` costs us the ability
        // to edit it later, and must not be reported as a failed delivery.
        Ok(v.get("ts")
            .and_then(|t| t.as_str())
            .filter(|t| !t.is_empty())
            .map(|ts| ExternalRef::Slack {
                channel_id: channel.to_string(),
                ts: ts.to_string(),
            }))
    }

    async fn update_message(&self, channel: &str, ts: &str, text: &str) -> Result<(), SlackError> {
        self.call(
            "chat.update",
            serde_json::json!({ "channel": channel, "ts": ts, "text": text }),
        )
        .await
        .map(|_| ())
    }
}

/// Slack projector egress (Cluster 309, made durable in 377.2): relay a Maidan
/// message posted in a linked thread out to its Slack channel — by *enqueueing* it
/// on the egress outbox, which [`egress_worker`](crate::egress_worker) drains with
/// retry/backoff. Until 377.2 this posted inline and a transient failure dropped
/// the message.
///
/// No-op unless a [`SlackSender`] is configured (the worker only runs then, so
/// queueing without one would pile up rows nothing drains); **skips messages that
/// originated in Slack** (the `metadata.slack` tag from the ingress, Cluster 308)
/// so a projected inbound message is never echoed back — loop prevention.
///
/// `log_id` is the `maidan_events` row being routed. It is the dedup key together
/// with the target: every replica runs the notification router, so all of them
/// enqueue and exactly one row survives.
pub async fn route_message_to_slack(
    state: &AppState,
    log_id: i64,
    thread_id: maidan_types::ThreadId,
    message: &maidan_types::Message,
) {
    if state.slack_sender.is_none() {
        return;
    }
    if message.metadata.get("slack").is_some() {
        return; // originated in Slack — don't echo it back
    }
    let link = match state
        .store
        .get_slack_channel_link_by_thread(thread_id)
        .await
    {
        Ok(Some(l)) if l.disabled_at.is_none() => l,
        // Disabled by an auth/config-class failure (Cluster 377.3): queueing into
        // a link a retry cannot fix only grows the dead-letter queue. Re-linking
        // turns it back on.
        Ok(Some(_)) => return,
        Ok(None) => return, // thread not linked to a Slack channel
        Err(err) => {
            tracing::warn!(error = %err, "slack egress: link lookup failed");
            return;
        }
    };
    let queued = state
        .store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: link.workspace_id,
            thread_id,
            source_log_id: log_id,
            target: EgressTarget::Slack {
                channel_id: link.slack_channel_id,
            },
            body: message.body.clone(),
            kind: EgressKind::Projector,
        })
        .await;
    if let Err(err) = queued {
        tracing::warn!(error = %err, "slack egress: enqueue failed");
    }
}

/// `POST /workspaces/:wid/slack-links` (Cluster 346) — link a Slack channel to a
/// Maidan thread so the projector can bridge messages both ways. The link's
/// `channel_id`/`workspace_id` come from resolving the thread (so they can't
/// disagree with it); the caller supplies only the Slack channel id, thread, and
/// the member that relayed Slack messages are attributed to. `workspace:write` +
/// access to the thread. Upserts (re-linking a Slack channel replaces its link).
pub async fn link_slack_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<LinkSlackChannel>,
) -> ApiResult<(StatusCode, Json<SlackChannelLink>)> {
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
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: body.slack_channel_id,
            workspace_id: scope.workspace_id,
            channel_id: scope.channel_id,
            thread_id: ThreadId(body.thread_id),
            member_id: MemberId(body.member_id),
        })
        .await?;
    Ok((StatusCode::CREATED, Json(link)))
}

/// `GET /workspaces/:wid/slack-links` (Cluster 346) — the workspace's Slack
/// channel links. `workspace:read`.
pub async fn list_slack_channel_links(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<SlackChannelLink>>> {
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, WorkspaceId(wid))?;
    Ok(Json(
        state
            .store
            .list_slack_channel_links(WorkspaceId(wid))
            .await?,
    ))
}

/// `DELETE /workspaces/:wid/slack-links/:slack_channel_id` (Cluster 346) — remove
/// a Slack channel link. `workspace:write`. `404` if the link doesn't exist.
pub async fn unlink_slack_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, slack_channel_id)): Path<(uuid::Uuid, String)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, WorkspaceId(wid))?;
    // Scope the delete to this workspace: only unlink if the link belongs to it.
    match state
        .store
        .get_slack_channel_link(&slack_channel_id)
        .await?
    {
        Some(link) if link.workspace_id == WorkspaceId(wid) => {}
        _ => return Err(crate::error::ApiError::NotFound),
    }
    if state.store.unlink_slack_channel(&slack_channel_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(crate::error::ApiError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, ts: &str, body: &str) -> String {
        slack_signature(secret, ts, body)
    }

    #[test]
    fn valid_signature_within_the_window_verifies() {
        let sig = sign("shhh", "1000", "body=1");
        assert!(verify_slack_signature("shhh", "1000", "body=1", &sig, 1010));
    }

    #[test]
    fn a_tampered_body_or_wrong_secret_fails() {
        let sig = sign("shhh", "1000", "body=1");
        assert!(!verify_slack_signature(
            "shhh", "1000", "body=2", &sig, 1010
        ));
        assert!(!verify_slack_signature(
            "other", "1000", "body=1", &sig, 1010
        ));
    }

    #[test]
    fn a_stale_timestamp_is_rejected() {
        let sig = sign("shhh", "1000", "body=1");
        assert!(!verify_slack_signature(
            "shhh",
            "1000",
            "body=1",
            &sig,
            1000 + 400
        ));
    }

    #[test]
    fn a_non_numeric_timestamp_is_rejected() {
        assert!(!verify_slack_signature(
            "shhh",
            "nope",
            "body=1",
            "v0=deadbeef",
            1000
        ));
    }

    #[test]
    fn credential_and_destination_errors_are_misconfigurations() {
        for code in [
            "invalid_auth",
            "token_revoked",
            "missing_scope",
            "channel_not_found",
            "not_in_channel",
            "is_archived",
        ] {
            assert!(
                SlackError::Api(code.into()).is_misconfiguration(),
                "{code} should disable the link"
            );
        }
    }

    #[test]
    fn transient_errors_are_not_misconfigurations() {
        // A rate limit or a Slack outage must keep retrying — disabling the link
        // over one would take an operator's re-link to undo.
        for code in ["ratelimited", "fatal_error", "service_unavailable"] {
            assert!(
                !SlackError::Api(code.into()).is_misconfiguration(),
                "{code} should be retried"
            );
        }
        assert!(!SlackError::Http("connection reset".into()).is_misconfiguration());
        // An error Slack adds later is retried, not treated as fatal.
        assert!(!SlackError::Api("some_new_code".into()).is_misconfiguration());
    }
}
