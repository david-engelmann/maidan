//! Named-secret MCP tools. List and resolve are `secret:read`. Create and
//! delete are `secret:admin` and call `create_secret_audited` /
//! `delete_secret_audited`, the same store functions as REST. The value
//! crosses the wire only on create and resolve, never in the event log.
//!
//! The secret-egress allowlist is managed here as over REST: the hosts the
//! egress broker may substitute this workspace's secret values for. Adding one
//! needs `secret:read` too, since a listed host receives the values.

use std::sync::Arc;

use maidan_auth::{
    capability::SECRET_READ, decrypt_peer_secret_rotating, encrypt_peer_secret, AuthContext,
};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateSecretArgs {
    name: String,
    value: String,
}

/// Store a named secret. Twin of `POST /workspaces/{wid}/secrets` (`secret:admin`).
/// Returns metadata only; the plaintext is not echoed.
pub(super) async fn create_secret(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CreateSecretArgs = serde_json::from_value(args.clone())?;
    if !maidan_types::is_valid_secret_name(&a.name) {
        return Err(McpError::InvalidParams(
            "secret name must be non-empty and use only [A-Za-z0-9_.-]".into(),
        ));
    }
    if a.value.is_empty() {
        return Err(McpError::InvalidParams(
            "secret value must not be empty".into(),
        ));
    }
    let Some(key) = server.encryption_key() else {
        return Err(McpError::Internal(
            "secret storage requires an encryption key configured on the server".into(),
        ));
    };
    let value_ciphertext =
        encrypt_peer_secret(&a.value, key).map_err(|e| McpError::Internal(e.to_string()))?;
    let actor = auth.actor_id;
    let secret = server
        .store
        .create_secret_audited(
            maidan_types::NewSecret {
                workspace_id: auth.workspace_id,
                name: a.name,
                value_ciphertext,
                created_by: auth.member_id,
            },
            Box::new(move |secret| maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(secret.workspace_id),
                actor_id: Some(actor),
                action: "secret.create".into(),
                target_kind: Some("secret".into()),
                target_id: Some(secret.id.0),
                metadata: json!({
                    "workspace_id": secret.workspace_id.0,
                    "name": secret.name,
                    "surface": "mcp",
                }),
            }),
        )
        .await?;
    Ok(content_json(&secret))
}

/// Delete a named secret. Twin of `DELETE /workspaces/{wid}/secrets/{name}`.
pub(super) async fn delete_secret(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ResolveArgs = serde_json::from_value(args.clone())?;
    let deleted = store
        .delete_secret_audited(
            auth.workspace_id,
            &a.name,
            maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(auth.workspace_id),
                actor_id: Some(auth.actor_id),
                action: "secret.delete".into(),
                target_kind: Some("secret".into()),
                target_id: None,
                metadata: json!({
                    "workspace_id": auth.workspace_id.0,
                    "name": a.name,
                    "surface": "mcp",
                }),
            },
        )
        .await?;
    if !deleted {
        return Err(McpError::NotFound);
    }
    Ok(content_json(&json!({ "name": a.name, "deleted": true })))
}
