//! Prepaid credits over MCP: the twins of `GET /workspaces/{id}/credits` and
//! `POST /workspaces/{id}/credits/top-ups`, so an agent told it is past its
//! allowance can read its balance and ask for a top-up without leaving its MCP
//! connection, and pay for itself. Both run the same
//! [`maidan_store::credit_terms`] code as the routes, and both stay open past
//! the allowance cutoff ([`super::REACHABLE_PAST_CUTOFF`]).

use maidan_auth::AuthContext;
use maidan_store::credit_terms::{self, TopUpError};
use maidan_types::WorkspaceId;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

impl From<TopUpError> for McpError {
    fn from(err: TopUpError) -> Self {
        match err {
            TopUpError::Disabled | TopUpError::CardNotConfigured | TopUpError::OutOfRange => {
                Self::InvalidParams(err.to_string())
            }
            TopUpError::Store(err) => err.into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreditsArgs {
    workspace_id: uuid::Uuid,
}

/// The workspace's balance, open top-up intents, recent ledger and the
/// instance's overage prices. `workspace:read`, the caller's own workspace.
pub(super) async fn get_credits(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CreditsArgs = super::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    // The pre-dispatch gate passes an id it cannot parse; this one parsed.
    auth.ensure_workspace(workspace_id)?;
    let terms = server.credit_terms();
    let credits = credit_terms::credits(
        server.store.as_ref(),
        &terms,
        workspace_id,
        chrono::Utc::now(),
    )
    .await?;
    Ok(content_json(&credits))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TopUpArgs {
    workspace_id: uuid::Uuid,
    usd: i64,
    #[serde(default)]
    method: maidan_types::TopUpMethod,
}

/// Open a top-up intent: the exact USDC amount to send on Base, where, and a
/// wallet link. Moves no money. `workspace:read`, the caller's own workspace.
pub(super) async fn top_up_credits(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: TopUpArgs = super::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)?;
    let terms = server.credit_terms();
    let top_up = credit_terms::open_top_up(
        server.store.as_ref(),
        &terms,
        workspace_id,
        a.usd,
        a.method,
        chrono::Utc::now(),
    )
    .await?;
    Ok(content_json(&top_up))
}
