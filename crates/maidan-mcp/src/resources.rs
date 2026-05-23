//! MCP resources backed by the store. Resources are addressed by URI:
//!
//! - `maidan://workspaces/{id}` — workspace metadata.
//! - `maidan://channels/{id}` — channel metadata + recent messages count.
//! - `maidan://threads/{id}` — full thread transcript (up to 100 messages).

use std::sync::Arc;

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
    ]
}

pub async fn read(store: &Arc<dyn Store>, uri: &str) -> Result<Value, McpError> {
    let path = uri
        .strip_prefix(URI_PREFIX)
        .ok_or_else(|| McpError::InvalidParams(format!("uri must start with maidan://: {uri}")))?;
    let mut parts = path.splitn(2, '/');
    let kind = parts.next().unwrap_or("");
    let id_str = parts
        .next()
        .ok_or_else(|| McpError::InvalidParams("missing id segment".into()))?;
    let id = uuid::Uuid::parse_str(id_str)
        .map_err(|_| McpError::InvalidParams(format!("invalid uuid in uri: {id_str}")))?;

    let payload = match kind {
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
