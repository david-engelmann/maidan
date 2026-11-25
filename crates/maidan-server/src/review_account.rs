//! Review accounts: `POST /operator/review-accounts` gives the operator a
//! tenant to hand to a directory, a catalog or a health check that reviews
//! the hosted service. Sign-up needs a person's emailed one-time code, which
//! none of them can answer.
//!
//! An account is the demo sandbox's tenant (`crate::sandbox::mint_tenant`):
//! a workspace of origin `open`, so the free allowance cuts it off and the
//! instance's own GitHub, Slack and Linear credentials never serve it, with a
//! `#general` channel, the "Say hello" task, an admin and two agents. It is
//! named for its provider, holds only that sample work, and its three tokens
//! expire after `ttl_days`, so it can be left to lapse, or revoked sooner by
//! erasing the workspace. It lends the reviewer nothing of the instance, and
//! it does not depend on the demo's switches or count against its limit.

use axum::extract::State;
use axum::{http::StatusCode, Extension, Json};
use chrono::{DateTime, Duration, Utc};
use maidan_auth::{capability::OPERATOR_GLOBAL, AuthContext};
use maidan_types::{AuditScope, ChannelId, NewAuditEvent, ThreadId, WorkspaceId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::extract::ApiJson;
use crate::routes::{cap, ApiResult};
use crate::sandbox::{mint_tenant, MintedCredential, TenantSpec};
use crate::state::AppState;

/// Longest provider name, in characters.
const MAX_PROVIDER_CHARS: usize = 32;
/// How long the tokens live when the request names no number.
const DEFAULT_TTL_DAYS: i64 = 90;
/// The longest an account may live: short enough that a forgotten one lapses
/// on its own.
const MAX_TTL_DAYS: i64 = 365;

/// Body for `POST /operator/review-accounts`. Both fields are checked after the capability check, so a body of `{}` still reaches it.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MintReviewAccount {
    /// The directory or service the account is for: 1 to 32 characters of `a-z`, `0-9` and `-`.
    #[serde(default)]
    pub provider: String,
    /// How long the tokens live, 1 to 90 days. Defaults to 90.
    #[serde(default)]
    pub ttl_days: Option<i64>,
}

/// `POST /operator/review-accounts` response. The three tokens are shown once.
#[derive(Debug, Serialize, ToSchema)]
pub struct MintedReviewAccount {
    pub provider: String,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub task_id: ThreadId,
    /// When the tokens stop working.
    pub expires_at: DateTime<Utc>,
    /// The MCP endpoint an agent connects to, absolute.
    pub mcp_url: String,
    /// The console, absolute.
    pub console_url: String,
    // The person's token: every workspace capability, for the console.
    pub admin: MintedCredential,
    // The agent's token: `maidan.agent.worker`, for an MCP client.
    pub agent: MintedCredential,
    // A second agent's token, the same capability set, to review the first's work.
    pub reviewer: MintedCredential,
}

/// A provider name is a label and part of a workspace name and a token label,
/// so it is held to a shape that needs no escaping anywhere it lands.
fn checked_provider(raw: &str) -> Result<&str, ApiError> {
    let chars = raw.chars().count();
    if chars == 0
        || chars > MAX_PROVIDER_CHARS
        || !raw
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ApiError::BadRequest(format!(
            "provider must be 1 to {MAX_PROVIDER_CHARS} characters of a-z, 0-9 and -"
        )));
    }
    Ok(raw)
}

fn checked_ttl_days(raw: Option<i64>) -> Result<i64, ApiError> {
    let days = raw.unwrap_or(DEFAULT_TTL_DAYS);
    if !(1..=MAX_TTL_DAYS).contains(&days) {
        return Err(ApiError::BadRequest(format!(
            "ttl_days must be from 1 to {MAX_TTL_DAYS}"
        )));
    }
    Ok(days)
}

/// `POST /operator/review-accounts` — a tenant of sample work for one
/// provider's checks. `operator:global`. Each provider gets its own, so
/// revoking one reaches nobody else.
pub async fn mint_review_account(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<MintReviewAccount>,
) -> ApiResult<(StatusCode, Json<MintedReviewAccount>)> {
    let _ = (&auth, OPERATOR_GLOBAL);
    let provider = checked_provider(&body.provider)?;
    let ttl_days = checked_ttl_days(body.ttl_days)?;
    // The MCP URL it hands out is built on the public origin, so without one
    // there is nothing a reviewer could connect to.
    let Some(origin) = state.public_origin.as_deref() else {
        return Err(ApiError::Overloaded(
            "review accounts hand out this instance's URLs, so MAIDAN_PUBLIC_ORIGIN must be set \
             on it"
                .into(),
        ));
    };
    let origin = origin.trim_end_matches('/');
    let expires_at = Utc::now() + Duration::days(ttl_days);
    let minted = mint_tenant(
        &state,
        TenantSpec {
            workspace_name: format!("Review: {provider}"),
            channel_topic: format!(
                "Sample work for {provider}'s checks. Nothing here belongs to anyone."
            ),
            admin_label: format!("review {provider} admin"),
            agent_label: format!("review {provider} agent"),
            reviewer_label: format!("review {provider} reviewer"),
            audit_source: "review_account",
            expires_at,
            mcp_url: format!("{origin}/mcp/streamable"),
            console_url: format!("{origin}/ui/"),
        },
    )
    .await?;
    // A routine record of who asked for what; the token mints themselves
    // were audited in their own transactions.
    crate::audit::record(
        &state,
        NewAuditEvent {
            scope: AuditScope::Workspace(minted.workspace_id),
            actor_id: Some(auth.actor_id()),
            action: "review_account.made".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(minted.workspace_id.0),
            metadata: serde_json::json!({
                "provider": provider,
                "expires_at": expires_at,
            }),
        },
    )
    .await;
    tracing::info!(workspace = %minted.workspace_id.0, provider, "review account minted");
    Ok((
        StatusCode::CREATED,
        Json(MintedReviewAccount {
            provider: provider.to_string(),
            workspace_id: minted.workspace_id,
            channel_id: minted.channel_id,
            task_id: minted.task_id,
            expires_at: minted.expires_at,
            mcp_url: minted.mcp_url,
            console_url: minted.console_url,
            admin: minted.admin,
            agent: minted.agent,
            reviewer: minted.reviewer,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_is_a_short_lowercase_label() {
        let longest = "a".repeat(MAX_PROVIDER_CHARS);
        for ok in ["glama", "openai-apps", "a", "x1-2", longest.as_str()] {
            assert_eq!(checked_provider(ok).ok(), Some(ok), "{ok}");
        }
        let too_long = "a".repeat(MAX_PROVIDER_CHARS + 1);
        for bad in [
            "",
            "Glama",
            "glama!",
            "gla ma",
            " glama",
            "gläma",
            "a/b",
            "a_b",
            too_long.as_str(),
        ] {
            assert!(checked_provider(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_lifetime_defaults_to_the_longest_and_refuses_beyond_it() {
        assert_eq!(checked_ttl_days(None).ok(), Some(90));
        assert_eq!(checked_ttl_days(Some(1)).ok(), Some(1));
        assert_eq!(checked_ttl_days(Some(90)).ok(), Some(90));
        for bad in [0, -1, 91, i64::MAX, i64::MIN] {
            assert!(checked_ttl_days(Some(bad)).is_err(), "{bad}");
        }
    }
}
