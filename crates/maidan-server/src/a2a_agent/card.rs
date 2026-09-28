//! The Agent Card (§8): public at `/.well-known/agent-card.json`, and the
//! extended card for authenticated clients (`GetExtendedAgentCard`).

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use maidan_a2a::{A2aError, A2A_PROTOCOL_VERSION};
use maidan_auth::capability::MESSAGE_POST;
use maidan_auth::AuthContext;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::error::denied;
use crate::state::AppState;

/// How long a client may reuse a fetched card. The card changes only when
/// the server restarts with new configuration.
const CARD_MAX_AGE_SECS: u32 = 300;

/// One transport binding in `supportedInterfaces`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInterface {
    pub url: String,
    pub protocol_binding: String,
    pub protocol_version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
    pub extended_agent_card: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    pub url: String,
    pub organization: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
}

/// The A2A v1.0 Agent Card (§4.4.1).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub supported_interfaces: Vec<AgentInterface>,
    pub provider: AgentProvider,
    pub version: String,
    pub documentation_url: String,
    pub capabilities: AgentCapabilities,
    /// `SecurityScheme` objects by name (§4.5).
    pub security_schemes: Value,
    /// `SecurityRequirement` objects: any one satisfies the agent.
    pub security_requirements: Value,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: Vec<AgentSkill>,
}

/// Deployment inputs to the card's transport advertisement, read once at
/// startup.
#[derive(Debug, Clone)]
pub struct A2aCardConfig {
    /// Public origin (e.g. `https://maidan.example`) for absolute HTTP
    /// interface URLs. Unset: the URLs stay host-relative.
    pub public_origin: Option<String>,
    /// Advertised gRPC address (`host:port`). Set: a `GRPC` interface is
    /// added. Distinct from the bind address (`MAIDAN_A2A_GRPC_ADDR`).
    pub grpc_public_addr: Option<String>,
    /// When this configuration was read: the card's `Last-Modified`.
    pub issued_at: DateTime<Utc>,
}

impl Default for A2aCardConfig {
    fn default() -> Self {
        Self {
            public_origin: None,
            grpc_public_addr: None,
            issued_at: Utc::now(),
        }
    }
}

impl A2aCardConfig {
    pub fn from_env() -> Self {
        let non_empty = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
        Self {
            public_origin: non_empty("MAIDAN_A2A_PUBLIC_ORIGIN"),
            grpc_public_addr: non_empty("MAIDAN_A2A_GRPC_PUBLIC_ADDR"),
            issued_at: Utc::now(),
        }
    }

    fn url(&self, path: &str) -> String {
        match &self.public_origin {
            Some(origin) => format!("{}{path}", origin.trim_end_matches('/')),
            None => path.to_string(),
        }
    }

    pub fn card(&self) -> AgentCard {
        let interface = |url: String, binding: &str| AgentInterface {
            url,
            protocol_binding: binding.into(),
            protocol_version: A2A_PROTOCOL_VERSION.into(),
        };
        let mut supported_interfaces = vec![
            interface(self.url("/a2a/v1/rpc"), "JSONRPC"),
            interface(self.url("/a2a/v1"), "HTTP+JSON"),
        ];
        if let Some(grpc) = &self.grpc_public_addr {
            supported_interfaces.push(interface(grpc.clone(), "GRPC"));
        }
        AgentCard {
            name: "maidan".into(),
            description: "Maidan — the operating layer for teams of AI agents. Messages sent \
                          here land in a shared, durable workspace thread."
                .into(),
            supported_interfaces,
            provider: AgentProvider {
                url: "https://github.com/david-engelmann/maidan".into(),
                organization: "Maidan".into(),
            },
            version: crate::version().to_string(),
            documentation_url:
                "https://github.com/david-engelmann/maidan/blob/main/docs/Protocols.md".into(),
            capabilities: AgentCapabilities {
                streaming: true,
                push_notifications: true,
                extended_agent_card: true,
            },
            security_schemes: json!({
                "bearer": { "httpAuthSecurityScheme": {
                    "scheme": "Bearer",
                    "description": "A Maidan API token (maid_…) for a workspace member.",
                } }
            }),
            security_requirements: json!([{ "schemes": { "bearer": { "list": [] } } }]),
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into()],
            skills: vec![AgentSkill {
                id: "collaborate".into(),
                name: "Workspace collaboration".into(),
                description: "Post messages into shared workspace threads (a context is a \
                              thread) and follow the resulting tasks, including pending \
                              approval gates."
                    .into(),
                tags: vec!["messaging".into(), "tasks".into(), "collaboration".into()],
            }],
        }
    }
}

/// `GET /.well-known/agent-card.json`, cacheable: `Cache-Control`, an `ETag`
/// over the card's bytes (a matching `If-None-Match` answers 304) and
/// `Last-Modified`.
pub async fn agent_card(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let config = &state.a2a_card;
    let body = match serde_json::to_vec(&config.card()) {
        Ok(body) => body,
        Err(err) => {
            tracing::error!(error = %err, "agent card failed to serialize");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let etag = format!("\"{}\"", hex::encode(&Sha256::digest(&body)[..16]));
    let cache = [
        (
            header::CACHE_CONTROL,
            format!("public, max-age={CARD_MAX_AGE_SECS}"),
        ),
        (header::ETAG, etag.clone()),
        (
            header::LAST_MODIFIED,
            config
                .issued_at
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string(),
        ),
    ];
    let fresh = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .map(|t| t.trim().trim_start_matches("W/"))
                .any(|t| t == "*" || t == etag)
        });
    if fresh {
        return (StatusCode::NOT_MODIFIED, cache).into_response();
    }
    (
        cache,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        body,
    )
        .into_response()
}

/// `GetExtendedAgentCard`: the card for an authenticated client.
pub(super) fn extended(state: &AppState, auth: &AuthContext) -> Result<AgentCard, A2aError> {
    auth.require_capability(MESSAGE_POST).map_err(denied)?;
    Ok(state.a2a_card.card())
}
