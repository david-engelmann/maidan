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
use maidan_types::{MemberId, NewSlackChannelLink, SlackChannelLink, ThreadId, WorkspaceId};
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

/// Outbound Slack sender — `chat.postMessage` in production, a mock in tests.
#[async_trait::async_trait]
pub trait SlackSender: Send + Sync {
    async fn post_message(&self, channel: &str, text: &str) -> Result<(), SlackError>;
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
}

#[async_trait::async_trait]
impl SlackSender for SlackWebClient {
    async fn post_message(&self, channel: &str, text: &str) -> Result<(), SlackError> {
        let resp = self
            .http
            .post(format!("{}/api/chat.postMessage", self.base_url))
            .bearer_auth(&self.bot_token)
            .json(&serde_json::json!({ "channel": channel, "text": text }))
            .send()
            .await
            .map_err(|e| SlackError::Http(e.to_string()))?;
        // Slack returns HTTP 200 with `{"ok": false, "error": ...}` on logical errors.
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SlackError::Http(e.to_string()))?;
        if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
            Ok(())
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

/// Slack projector egress (Cluster 309): relay a Maidan message posted in a linked
/// thread out to its Slack channel. No-op unless a [`SlackSender`] is configured;
/// **skips messages that originated in Slack** (the `metadata.slack` tag from the
/// ingress, Cluster 308) so a projected inbound message is never echoed back —
/// loop prevention. Best-effort (a failed post is logged + metered, not retried).
pub async fn route_message_to_slack(
    state: &AppState,
    thread_id: maidan_types::ThreadId,
    message: &maidan_types::Message,
) {
    let Some(sender) = state.slack_sender.as_ref() else {
        return;
    };
    if message.metadata.get("slack").is_some() {
        return; // originated in Slack — don't echo it back
    }
    let link = match state
        .store
        .get_slack_channel_link_by_thread(thread_id)
        .await
    {
        Ok(Some(l)) => l,
        Ok(None) => return, // thread not linked to a Slack channel
        Err(err) => {
            tracing::warn!(error = %err, "slack egress: link lookup failed");
            return;
        }
    };
    match sender
        .post_message(&link.slack_channel_id, &message.body)
        .await
    {
        Ok(()) => crate::metrics::record_slack_egress("sent"),
        Err(err) => {
            tracing::warn!(error = %err, "slack egress: post failed");
            crate::metrics::record_slack_egress("failed");
        }
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
}
