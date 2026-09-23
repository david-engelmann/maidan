//! MCP resources backed by the store. Resources are addressed by URI:
//!
//! - `maidan://workspaces/{id}` — workspace metadata.
//! - `maidan://channels/{id}` — channel metadata + recent messages count.
//! - `maidan://threads/{id}` — full thread transcript (up to 100 messages).

use std::sync::Arc;

use maidan_artifacts::Sha256;
use maidan_store::Store;
use maidan_types::*;
use serde_json::{json, Value};

use crate::error::McpError;

const URI_PREFIX: &str = "maidan://";

/// The resources this server serves, as URI templates (`resources/templates/list`).
///
/// Every Maidan resource is addressed by an id, so all of them are templates.
/// They used to be returned from `resources/list` with the placeholder left in
/// the `uri` — which a spec client reads as a concrete resource it can fetch,
/// and cannot.
pub fn templates() -> Vec<Value> {
    [
        (
            "workspace",
            "maidan://workspaces/{id}",
            "Workspace metadata.",
        ),
        ("channel", "maidan://channels/{id}", "Channel metadata."),
        (
            "thread",
            "maidan://threads/{id}",
            "Full thread transcript (up to 100 messages).",
        ),
        (
            "artifact",
            "maidan://artifacts/{sha256}",
            "Artifact metadata and byte length (body omitted).",
        ),
    ]
    .into_iter()
    .map(|(name, uri_template, description)| {
        json!({
            "uriTemplate": uri_template,
            "name": name,
            "description": description,
            "mimeType": "application/json",
        })
    })
    .collect()
}

/// Concrete resources for `resources/list`: the caller's own workspace, the one
/// resource that exists without the caller naming an id. Everything else is
/// reached through [`templates`].
pub fn listed(auth: &maidan_auth::AuthContext) -> Vec<Value> {
    if auth.bypass {
        return Vec::new();
    }
    vec![json!({
        "uri": format!("{URI_PREFIX}workspaces/{}", auth.workspace_id.0),
        "name": "workspace",
        "description": "Your workspace's metadata.",
        "mimeType": "application/json",
    })]
}

pub async fn read(store: &Arc<dyn Store>, uri: &str) -> Result<Value, McpError> {
    let (kind, id_str) = parse_uri(uri)?;

    let payload = match kind {
        "artifacts" => {
            if id_str.len() != 64 {
                return Err(McpError::InvalidParams(
                    "artifact sha256 must be 64 hex chars".into(),
                ));
            }
            // Access is gated by the caller (server::resources_read →
            // artifact_ref_exists). `size_bytes` is
            // authoritative metadata, so the blob is never loaded just to
            // report its length.
            let meta = store.get_artifact_by_sha(id_str).await?;
            json!({
                "artifact": meta,
                "byte_length": meta.size_bytes,
            })
        }
        "workspaces" | "channels" | "threads" => {
            let id = uuid::Uuid::parse_str(id_str)
                .map_err(|_| McpError::InvalidParams(format!("invalid uuid in uri: {id_str}")))?;
            match kind {
                "workspaces" => {
                    let ws = store.get_workspace(WorkspaceId(id)).await?;
                    serde_json::to_value(&ws)?
                }
                "channels" => {
                    let ch = store.get_channel(ChannelId(id)).await?;
                    serde_json::to_value(&ch)?
                }
                "threads" => {
                    let thread = store.get_thread(ThreadId(id)).await?;
                    let messages = store.list_messages(ThreadId(id), 100).await?;
                    json!({
                        "thread": thread,
                        "messages": messages,
                    })
                }
                other => {
                    return Err(McpError::InvalidParams(format!(
                        "unknown resource kind: {other}"
                    )))
                }
            }
        }
        other => {
            return Err(McpError::InvalidParams(format!(
                "unknown resource kind: {other}"
            )));
        }
    };

    let text = serde_json::to_string(&payload)?;
    Ok(json!({
        "contents": [
            {
                "uri": uri,
                "mimeType": "application/json",
                "text": text
            }
        ]
    }))
}

pub fn validate_uri(uri: &str) -> Result<(), McpError> {
    let (kind, id_str) = parse_uri(uri)?;
    match kind {
        "artifacts" => {
            if id_str.len() != 64 {
                return Err(McpError::InvalidParams(
                    "artifact sha256 must be 64 hex chars".into(),
                ));
            }
            let _ = Sha256::from_hex(id_str).map_err(|e| McpError::InvalidParams(e.to_string()))?;
        }
        "workspaces" | "channels" | "threads" => {
            let _ = uuid::Uuid::parse_str(id_str)
                .map_err(|_| McpError::InvalidParams(format!("invalid uuid in uri: {id_str}")))?;
        }
        other => {
            return Err(McpError::InvalidParams(format!(
                "unknown resource kind: {other}"
            )));
        }
    }
    Ok(())
}

fn parse_uri(uri: &str) -> Result<(&str, &str), McpError> {
    let path = uri
        .strip_prefix(URI_PREFIX)
        .ok_or_else(|| McpError::InvalidParams(format!("uri must start with maidan://: {uri}")))?;
    let mut parts = path.splitn(2, '/');
    let kind = parts.next().unwrap_or("");
    let id_str = parts
        .next()
        .ok_or_else(|| McpError::InvalidParams("missing id segment".into()))?;
    Ok((kind, id_str))
}
