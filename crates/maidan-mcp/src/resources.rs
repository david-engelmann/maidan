//! MCP resources backed by the store. Resources are addressed by URI:
//!
//! - `maidan://workspaces/{id}` — workspace metadata.
//! - `maidan://channels/{id}` — channel metadata + recent messages count.
//! - `maidan://threads/{id}` — full thread transcript (up to 100 messages).

use std::sync::Arc;

use maidan_artifacts::{ArtifactStore, Sha256};
use maidan_store::Store;
use maidan_types::*;
use serde_json::{json, Value};

use crate::error::McpError;

const URI_PREFIX: &str = "maidan://";

pub fn catalog() -> Vec<Value> {
    vec![
        json!({
            "uri": "maidan://workspaces/{id}",
            "name": "workspace",
            "description": "Workspace metadata."
        }),
        json!({
            "uri": "maidan://channels/{id}",
            "name": "channel",
            "description": "Channel metadata."
        }),
        json!({
            "uri": "maidan://threads/{id}",
            "name": "thread",
            "description": "Full thread transcript (up to 100 messages)."
        }),
        json!({
            "uri": "maidan://artifacts/{sha256}",
            "name": "artifact",
            "description": "Artifact metadata and byte length (body omitted)."
        }),
    ]
}

pub async fn read(
    store: &Arc<dyn Store>,
    artifacts: &Arc<dyn ArtifactStore>,
    uri: &str,
) -> Result<Value, McpError> {
    let (kind, id_str) = parse_uri(uri)?;

    let payload = match kind {
        "artifacts" => {
            if id_str.len() != 64 {
                return Err(McpError::InvalidParams(
                    "artifact sha256 must be 64 hex chars".into(),
                ));
            }
            let meta = store.get_artifact_by_sha(id_str).await?;
            let sha =
                Sha256::from_hex(id_str).map_err(|e| McpError::InvalidParams(e.to_string()))?;
            let body = artifacts
                .get(&sha)
                .await
                .map_err(|e| McpError::Internal(e.to_string()))?;
            json!({
                "artifact": meta,
                "byte_length": body.len(),
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
                _ => unreachable!(),
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
