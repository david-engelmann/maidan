//! The demo's per-visitor sandbox: `POST /sandbox` gives a stranger their own
//! throwaway workspace and two tokens, with no sign-up, so they can connect an
//! agent to try.maidan.world within a minute.
//!
//! On only with `MAIDAN_DEMO_SANDBOX=1`, and only on an instance that is a
//! throwaway as a whole: production mode needs `MAIDAN_DEMO_INSTANCE=1`
//! beside it, sign-up and `AUTH_DISABLED` refuse it, and a public origin must
//! be set for the MCP URL the answer gives. Off, the route answers 404.
//!
//! Each call makes a workspace (origin `open`, so the free allowance cuts it
//! off), a `#general` channel, the sign-up's "Say hello" starter task with its
//! opening message, an admin and an agent, and returns both secrets once.
//! Both tokens expire in 24 hours; the demo's daily reset deletes everything
//! sooner or later anyway. The route is charged per address by the
//! rate-limit middleware, takes Turnstile when the instance has it, refuses a
//! cross-site browser post, and stops at `MAIDAN_DEMO_SANDBOX_MAX` workspaces.
//! Each sandbox is an ordinary tenant, so tenant isolation keeps one visitor
//! out of another's.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use maidan_auth::{
    capability::{self, AUDIT_READ_GLOBAL, FEDERATION_ADMIN, FEDERATION_INGEST, OPERATOR_GLOBAL},
    capability_set::AGENT_WORKER,
    hash_secret, TokenSecret,
};
use maidan_types::{
    ApiToken, AuditScope, ChannelId, Member, MemberId, NewAuditEvent, NewChannel, NewMessage,
    NewThread, ThreadId, WorkspaceId, WorkspaceOrigin,
};
use serde::{Deserialize, Serialize};

use crate::config::ConfigError;
use crate::error::ApiError;
use crate::extract::ApiJson;
use crate::oidc::{STARTER_TASK_DESCRIPTION, STARTER_TASK_MESSAGE, STARTER_TASK_TITLE};
use crate::state::AppState;

/// Turns the mint on.
pub const ENV: &str = "MAIDAN_DEMO_SANDBOX";
/// The marker that this whole instance is a disposable demo. Production mode
/// refuses the mint without it, so a real deployment cannot hand out
/// workspaces to strangers by setting one variable.
pub const DEMO_INSTANCE_ENV: &str = "MAIDAN_DEMO_INSTANCE";
/// The most workspaces the instance holds before the mint refuses.
pub const MAX_ENV: &str = "MAIDAN_DEMO_SANDBOX_MAX";
/// Two hundred sandboxes a day is far past what the demo expects, and at the
/// demo's 4 MiB stored-bytes allowance each still fits the container's disk.
pub const DEFAULT_MAX: i64 = 200;
/// How long both tokens live.
pub const TOKEN_TTL_HOURS: i64 = 24;
/// The channel a sandbox opens on.
pub const CHANNEL: &str = "general";

#[derive(Debug, Clone)]
pub struct SandboxSettings {
    pub max_sandboxes: i64,
    /// The MCP endpoint an agent connects to, absolute.
    pub mcp_url: String,
    /// The console, absolute.
    pub console_url: String,
}

fn truthy(raw: Option<&str>) -> bool {
    matches!(raw, Some("1") | Some("true") | Some("TRUE"))
}

/// Whether the mint may be on. Pure over its inputs so it is tested without
/// touching the process environment.
pub(crate) fn validate(
    production: bool,
    demo_instance: bool,
    auth_disabled: bool,
    signup: bool,
    public_origin: Option<&str>,
) -> Result<(), ConfigError> {
    if production && !demo_instance {
        return Err(ConfigError::Invalid(
            ENV,
            format!(
                "hands workspaces to anyone who asks; under MAIDAN_ENV=production it needs \
                 {DEMO_INSTANCE_ENV}=1 to say the whole instance is a throwaway demo"
            ),
        ));
    }
    if auth_disabled {
        return Err(ConfigError::Invalid(
            ENV,
            "cannot be set with AUTH_DISABLED, under which no sandbox is isolated".into(),
        ));
    }
    if signup {
        return Err(ConfigError::Invalid(
            ENV,
            "cannot be set with MAIDAN_SIGNUP: an instance with real sign-ups is not a demo".into(),
        ));
    }
    if public_origin.is_none() {
        return Err(ConfigError::Invalid(
            ENV,
            "needs MAIDAN_PUBLIC_ORIGIN, which the MCP URL it hands out is built on".into(),
        ));
    }
    Ok(())
}

fn parse_max(raw: Option<&str>) -> Result<i64, ConfigError> {
    match raw.map(str::trim) {
        None | Some("") => Ok(DEFAULT_MAX),
        Some(v) => match v.parse::<i64>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err(ConfigError::Invalid(
                MAX_ENV,
                "must be a whole number above zero".into(),
            )),
        },
    }
}

impl SandboxSettings {
    /// Off (`None`) unless `MAIDAN_DEMO_SANDBOX` is set. `public_origin` is
    /// the normalized `MAIDAN_PUBLIC_ORIGIN`.
    pub fn from_env(
        public_origin: Option<&str>,
        auth_disabled: bool,
    ) -> Result<Option<Self>, ConfigError> {
        let var = |name| std::env::var(name).ok();
        if !truthy(var(ENV).as_deref()) {
            return Ok(None);
        }
        validate(
            crate::config::is_production(),
            truthy(var(DEMO_INSTANCE_ENV).as_deref()),
            auth_disabled,
            truthy(var("MAIDAN_SIGNUP").as_deref()),
            public_origin,
        )?;
        let origin = public_origin.unwrap_or_default().trim_end_matches('/');
        Ok(Some(Self {
            max_sandboxes: parse_max(var(MAX_ENV).as_deref())?,
            mcp_url: format!("{origin}/mcp/streamable"),
            console_url: format!("{origin}/ui/"),
        }))
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct MintSandboxRequest {
    /// The Turnstile widget's token. Required when the instance runs
    /// Turnstile, ignored otherwise.
    #[serde(default)]
    pub turnstile_token: Option<String>,
}

/// One member of a new sandbox and its token, shown once.
#[derive(Debug, Serialize)]
pub struct SandboxCredential {
    pub member_id: MemberId,
    pub handle: String,
    pub token: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MintedSandbox {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub task_id: ThreadId,
    /// When both tokens stop working. The demo's daily reset may delete the
    /// sandbox before then.
    pub expires_at: DateTime<Utc>,
    pub mcp_url: String,
    pub console_url: String,
    /// The person's token: every workspace capability, for the console.
    pub admin: SandboxCredential,
    /// The agent's token: `maidan.agent.worker`, for an MCP client.
    pub agent: SandboxCredential,
}

/// What a sandbox admin holds: everything in its own workspace, and nothing
/// that reads across tenants or federates with another server.
fn sandbox_admin_capabilities() -> Vec<String> {
    capability::all()
        .into_iter()
        .filter(|cap| {
            ![
                OPERATOR_GLOBAL,
                AUDIT_READ_GLOBAL,
                FEDERATION_ADMIN,
                FEDERATION_INGEST,
            ]
            .contains(&cap.as_str())
        })
        .collect()
}

fn token_audit(actor: MemberId, source: &'static str) -> maidan_store::AuditFor<ApiToken> {
    Box::new(move |record: &ApiToken| NewAuditEvent {
        scope: AuditScope::Workspace(record.workspace_id),
        actor_id: Some(actor),
        action: "token.mint".into(),
        target_kind: Some("api_token".into()),
        target_id: Some(record.id.0),
        metadata: serde_json::json!({
            "workspace_id": record.workspace_id.0,
            "subject_member_id": record.member_id.0,
            "capabilities": record.capabilities.clone(),
            "expires_at": record.expires_at,
            "source": source,
        }),
    })
}

/// `POST /sandbox` — a fresh throwaway workspace for whoever asks. Public on
/// a demo instance, 404 everywhere else.
pub async fn mint(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<MintSandboxRequest>,
) -> Result<(StatusCode, Json<MintedSandbox>), ApiError> {
    let Some(settings) = state.sandbox.clone() else {
        return Err(ApiError::NotFound);
    };
    // Another site must not spend its visitors' address budgets, or fill the
    // demo, through their browsers.
    if crate::routes::cross_site(&headers, state.public_origin.as_deref()) {
        return Err(ApiError::Forbidden(
            "a sandbox is started from this site only".into(),
        ));
    }
    if let Some(turnstile) = state.turnstile.as_deref() {
        crate::turnstile::verify(
            turnstile,
            body.turnstile_token.as_deref(),
            crate::turnstile::Action::Sandbox,
        )
        .await?;
    }
    // Two mints racing past this check can each make one more; the address
    // limit keeps that overshoot to a handful.
    if state.store.count_workspaces().await? >= settings.max_sandboxes {
        return Err(ApiError::Overloaded(
            "the demo is full until its daily reset; try again later".into(),
        ));
    }
    mint_sandbox(&state, &settings).await
}

async fn mint_sandbox(
    state: &AppState,
    settings: &Arc<SandboxSettings>,
) -> Result<(StatusCode, Json<MintedSandbox>), ApiError> {
    let expires_at = Utc::now() + Duration::hours(TOKEN_TTL_HOURS);

    let admin_secret = TokenSecret::generate();
    let provisioned = state
        .store
        .provision_workspace(
            maidan_store::store::NewProvisionedTenant {
                name: "Sandbox".into(),
                admin_handle: "you".into(),
                token_hash: hash_secret(admin_secret.as_str()),
                token_label: Some("sandbox admin".into()),
                capabilities: sandbox_admin_capabilities(),
                token_expires_at: Some(expires_at),
                origin: WorkspaceOrigin::Open,
            },
            Box::new(|record: &ApiToken| token_audit(record.member_id, "demo-sandbox")(record)),
        )
        .await?;
    for stored in provisioned.events {
        crate::routes::publish_stored(state, stored).await;
    }
    let workspace_id = provisioned.workspace.id;
    let admin = provisioned.member;

    let (channel, stored) = state
        .store
        .create_channel_with_event(NewChannel {
            workspace_id,
            name: CHANNEL.into(),
            topic: Some("Your sandbox. It is deleted within 24 hours.".into()),
            private: false,
        })
        .await?;
    crate::routes::publish_stored(state, stored).await;

    let agent_secret = TokenSecret::generate();
    let agent_caps =
        maidan_auth::capability_set::expand_set(AGENT_WORKER).map_err(ApiError::Internal)?;
    let actor = admin.id;
    let agent = state
        .store
        .create_agent_with_token(
            maidan_store::store::NewAgentWithToken {
                workspace_id,
                handle: "agent".into(),
                display_name: Some("Your agent".into()),
                token_hash: hash_secret(agent_secret.as_str()),
                token_label: Some("sandbox agent".into()),
                capabilities: agent_caps,
                expires_at: Some(expires_at),
            },
            Box::new(move |member: &Member| NewAuditEvent {
                scope: AuditScope::Workspace(member.workspace_id),
                actor_id: Some(actor),
                action: "member.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(member.id.0),
                metadata: serde_json::json!({
                    "workspace_id": member.workspace_id.0,
                    "handle": member.handle,
                    "kind": member.kind,
                    "source": "demo-sandbox",
                }),
            }),
            token_audit(actor, "demo-sandbox"),
        )
        .await?;
    crate::routes::publish_stored(state, agent.event).await;

    let (task, stored) = state
        .store
        .create_thread_with_footprint(
            NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some(STARTER_TASK_TITLE.into()),
                description: Some(STARTER_TASK_DESCRIPTION.into()),
                created_by: Some(admin.id),
            },
            None,
            None,
            None,
        )
        .await?;
    crate::routes::publish_stored(state, stored).await;
    // The person owns the task, so the review the agent asks for reaches them.
    state
        .store
        .set_thread_owner(task.id, Some(admin.id))
        .await?;
    // The opening message is what walks a freshly connected agent through
    // the loop, so a visitor gets the one a sign-up does.
    let (_, stored) = state
        .store
        .post_message_with_event(
            NewMessage {
                thread_id: task.id,
                author_id: admin.id,
                body: STARTER_TASK_MESSAGE.into(),
                metadata: serde_json::json!({ "starter": true }),
                content: None,
            },
            None,
        )
        .await?;
    crate::routes::publish_stored(state, stored).await;

    tracing::info!(workspace = %workspace_id.0, "demo sandbox minted");
    Ok((
        StatusCode::CREATED,
        Json(MintedSandbox {
            workspace_id,
            channel_id: channel.id,
            task_id: task.id,
            expires_at,
            mcp_url: settings.mcp_url.clone(),
            console_url: settings.console_url.clone(),
            admin: SandboxCredential {
                member_id: admin.id,
                handle: admin.handle,
                token: admin_secret.as_str().to_string(),
                capabilities: provisioned.token.capabilities,
            },
            agent: SandboxCredential {
                member_id: agent.member.id,
                handle: agent.member.handle,
                token: agent_secret.as_str().to_string(),
                capabilities: agent.token.capabilities,
            },
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_needs_the_demo_marker() {
        let origin = Some("https://try.example");
        assert!(validate(true, false, false, false, origin).is_err());
        assert!(validate(true, true, false, false, origin).is_ok());
        assert!(validate(false, false, false, false, origin).is_ok());
    }

    #[test]
    fn the_mint_is_refused_beside_auth_disabled_signup_or_no_origin() {
        let origin = Some("https://try.example");
        assert!(validate(false, false, true, false, origin).is_err());
        assert!(validate(false, false, false, true, origin).is_err());
        assert!(validate(false, false, false, false, None).is_err());
    }

    #[test]
    fn the_cap_defaults_and_refuses_nonsense() {
        assert_eq!(parse_max(None).ok(), Some(DEFAULT_MAX));
        assert_eq!(parse_max(Some("5")).ok(), Some(5));
        assert!(parse_max(Some("0")).is_err());
        assert!(parse_max(Some("-1")).is_err());
        assert!(parse_max(Some("many")).is_err());
    }

    #[test]
    fn a_sandbox_admin_cannot_reach_across_tenants_or_federate() {
        let caps = sandbox_admin_capabilities();
        for global in [
            OPERATOR_GLOBAL,
            AUDIT_READ_GLOBAL,
            FEDERATION_ADMIN,
            FEDERATION_INGEST,
        ] {
            assert!(!caps.iter().any(|c| c == global), "{global}");
        }
        assert!(caps.iter().any(|c| c == capability::TOKEN_ADMIN));
    }
}
