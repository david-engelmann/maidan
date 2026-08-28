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
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

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
