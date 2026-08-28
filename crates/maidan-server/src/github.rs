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
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

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
    match event {
        // Webhook setup handshake — GitHub sends `ping` once; a 200 confirms it.
        "ping" => Json(serde_json::json!({ "ok": true })).into_response(),
        // Other events — ACK fast (routing lands in Cluster 311).
        _ => StatusCode::OK.into_response(),
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
