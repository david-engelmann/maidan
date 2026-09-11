//! Named-secret MCP tools (Cluster 371.3, Wave 2 #19): an agent lists a
//! workspace's secrets (metadata) and **resolves** one by name — the "Pi fetches
//! at exec" path. The REST twin is Cluster 371.2. Both are `secret:read`; the
//! value crosses the wire only on resolve (decrypted with the server's key),
//! never in the event log. Minting/rotating/deleting stay REST-only (`secret:admin`).

use std::sync::Arc;

use maidan_auth::{decrypt_peer_secret_rotating, AuthContext};
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
struct ResolveArgs {
    name: String,
}

/// Resolve a named secret to its value (Cluster 371.3) — "Pi fetches at exec".
/// Needs the server's at-rest key to decrypt; `NotFound` when the name is unknown.
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
    Ok(content_json(&json!({ "name": a.name, "value": value })))
}
