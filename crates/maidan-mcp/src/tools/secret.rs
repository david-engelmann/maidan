//! Named-secret MCP tools: an agent lists a workspace's secrets (metadata) and
//! **resolves** one by name — the "consumer fetches at exec" path, mirroring
//! the REST surface. Both are `secret:read`; the value crosses the wire
//! only on resolve (decrypted with the server's key), never in the event log.
//! Minting/rotating/deleting stay REST-only (`secret:admin`).
//!
//! The secret-egress allowlist is managed here as over REST: the hosts the
//! egress broker may substitute this workspace's secret values for. Adding one
//! needs `secret:read` too, since a listed host receives the values.

use std::sync::Arc;

use maidan_auth::{capability::SECRET_READ, decrypt_peer_secret_rotating, AuthContext};
use maidan_store::Store;
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

/// List the caller's workspace's secrets (metadata only — never the value).
pub(super) async fn list_secrets(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let secrets = store.list_secrets(auth.workspace_id).await?;
    Ok(content_json(&secrets))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveArgs {
    name: String,
}

/// Resolve a named secret to its value — "a consumer fetches at exec". Needs
/// the server's at-rest key to decrypt; `NotFound` when the name is unknown.
pub(super) async fn resolve_secret(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ResolveArgs = serde_json::from_value(args.clone())?;
    let Some(key) = server.encryption_key() else {
        return Err(McpError::Internal(
            "secret storage requires an encryption key configured on the server".into(),
        ));
    };
    let ciphertext = server
        .store
        .get_secret_ciphertext(auth.workspace_id, &a.name)
        .await?
        .ok_or(McpError::NotFound)?;
    let value = decrypt_peer_secret_rotating(&ciphertext, key)
        .map_err(|e| McpError::Internal(format!("secret decrypt failed: {e}")))?;
    // Recorded before the value is released, and withheld if it cannot be.
    server
        .store
        .append_audit(maidan_types::NewAuditEvent {
            scope: maidan_types::AuditScope::Workspace(auth.workspace_id),
            actor_id: Some(auth.actor_id),
            action: "secret.resolve".into(),
            target_kind: Some("secret".into()),
            target_id: None,
            metadata: json!({
                "workspace_id": auth.workspace_id.0,
                "name": a.name,
                "surface": "mcp",
            }),
        })
        .await?;
    Ok(content_json(&json!({ "name": a.name, "value": value })))
}

/// The hosts trusted with the caller's workspace's secret values.
pub(super) async fn list_secret_egress_hosts(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let hosts = store.list_secret_egress_hosts(auth.workspace_id).await?;
    Ok(content_json(&hosts))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostArgs {
    host: String,
}

/// Trust a host with the workspace's secret values. Idempotent; refuses a host
/// that is not a bare hostname or is outside the instance ceiling.
pub(super) async fn allow_secret_egress_host(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    auth.require_capability(SECRET_READ)?;
    let a: HostArgs = serde_json::from_value(args.clone())?;
    let host = maidan_types::normalize_secret_egress_host(&a.host)
        .map_err(|why| McpError::InvalidParams(why.to_string()))?;
    if !maidan_types::within_secret_egress_ceiling(&host, server.secret_egress_ceiling()) {
        return Err(McpError::InvalidParams(
            "host is outside this instance's secret-egress ceiling (MAIDAN_SECRET_EGRESS_ALLOWLIST)"
                .into(),
        ));
    }
    let actor = auth.actor_id;
    let entry = server
        .store
        .allow_secret_egress_host_audited(
            maidan_types::NewSecretEgressHost {
                workspace_id: auth.workspace_id,
                host,
            },
            Box::new(move |entry| maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(entry.workspace_id),
                actor_id: Some(actor),
                action: "secret_egress_host.allow".into(),
                target_kind: Some("secret_egress_host".into()),
                target_id: None,
                metadata: json!({
                    "workspace_id": entry.workspace_id.0,
                    "host": entry.host,
                    "surface": "mcp",
                }),
            }),
        )
        .await?;
    Ok(content_json(&entry))
}

/// Stop trusting a host. `NotFound` when it was not listed.
pub(super) async fn revoke_secret_egress_host(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: HostArgs = serde_json::from_value(args.clone())?;
    let host = a.host.to_ascii_lowercase();
    let revoked = store
        .revoke_secret_egress_host_audited(
            auth.workspace_id,
            &host,
            maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(auth.workspace_id),
                actor_id: Some(auth.actor_id),
                action: "secret_egress_host.revoke".into(),
                target_kind: Some("secret_egress_host".into()),
                target_id: None,
                metadata: json!({
                    "workspace_id": auth.workspace_id.0,
                    "host": host,
                    "surface": "mcp",
                }),
            },
        )
        .await?;
    if !revoked {
        return Err(McpError::NotFound);
    }
    Ok(content_json(&json!({ "host": host, "revoked": true })))
}
