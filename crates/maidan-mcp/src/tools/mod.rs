//! MCP tools backed by [`maidan_store::Store`]. Each tool has a JSON
//! schema (input shape) and a dispatcher that decodes args, calls the
//! store, and returns a JSON result.
//!
//! The per-tool handlers are organized by domain in the submodules
//! below; the three entry points (`required_capability`, `catalog`,
//! `dispatch`) and the shared [`content_json`] helper live here.

use maidan_auth::capability::{
    ARTIFACT_UPLOAD, MESSAGE_POST, SEARCH_QUERY, WORKSPACE_READ, WORKSPACE_WRITE,
};
use maidan_auth::AuthContext;
use serde_json::{json, Value};

use crate::error::McpError;

mod artifact;
mod automation;
mod catalog;
mod channel;
mod member;
mod message;
mod reference;
mod search;
mod social;
mod thread;

pub use catalog::catalog;

pub fn required_capability(name: &str) -> Result<&'static str, McpError> {
    match name {
        "list_channels"
        | "list_threads"
        | "list_messages"
        | "list_dm_conversations"
        | "list_reactions"
        | "list_pins"
        | "get_artifact_metadata"
        | "get_thread_context"
        | "get_workspace_context"
        | "summarize_thread"
        | "list_mentions"
        | "get_inbox"
        | "mark_inbox_read" => Ok(WORKSPACE_READ),
        "open_dm_conversation" | "post_dm_message" | "post_message" | "edit_message" => {
            Ok(MESSAGE_POST)
        }
        "record_mention" | "cast_vote" | "add_reaction" | "remove_reaction" | "pin_message"
        | "unpin_message" | "add_reference" => Ok(WORKSPACE_WRITE),
        "upload_artifact"
        | "begin_artifact_multipart"
        | "upload_artifact_multipart_part"
        | "complete_artifact_multipart"
        | "abort_artifact_multipart" => Ok(ARTIFACT_UPLOAD),
        "search_messages" => Ok(SEARCH_QUERY),
        "register_slash_command" => Ok(WORKSPACE_WRITE),
        "list_slash_commands" => Ok(WORKSPACE_READ),
        "register_fsm_hook" => Ok(WORKSPACE_WRITE),
        "list_fsm_hooks" => Ok(WORKSPACE_READ),
        other => Err(McpError::MethodNotFound(format!("tools/{other}"))),
    }
}

/// Pre-dispatch per-channel authorization for point-access content tools
/// (Cluster 161). Bypass callers pass through; DM tools rely on their own
/// participant checks (the `__dm__` channel is exempt in `ensure_*`); aggregate
/// reads (`list_channels` / `get_workspace_context` / `search_messages`) filter
/// their result sets separately. A tool whose id arg is absent/malformed is left
/// to its handler's own decode error.
async fn enforce_channel_access(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    name: &str,
    args: &Value,
) -> Result<(), McpError> {
    if auth.bypass {
        return Ok(());
    }
    let store = server.store.as_ref();
    let field = |key: &str| -> Option<uuid::Uuid> {
        args.get(key)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
    };
    match name {
        "list_threads" => {
            if let Some(id) = field("channel_id") {
                maidan_auth::ensure_channel_access(store, auth, maidan_types::ChannelId(id))
                    .await?;
            }
        }
        "list_messages" | "post_message" | "get_thread_context" | "summarize_thread"
        | "pin_message" | "unpin_message" | "list_pins" => {
            if let Some(id) = field("thread_id") {
                maidan_auth::ensure_thread_access(store, auth, maidan_types::ThreadId(id)).await?;
            }
        }
        "edit_message" | "record_mention" | "cast_vote" | "add_reaction" | "remove_reaction"
        | "list_reactions" => {
            if let Some(id) = field("message_id") {
                maidan_auth::ensure_message_access(store, auth, maidan_types::MessageId(id))
                    .await?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub async fn dispatch(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    name: &str,
    args: &Value,
    session_id: Option<&str>,
) -> Result<Value, McpError> {
    enforce_channel_access(server, auth, name, args).await?;
    let store = &server.store;
    let artifacts = &server.artifacts;
    let search = &server.search;
    let embedding_provider = &server.embedding_provider;
    match name {
        "list_channels" => channel::list_channels(store, auth, args).await,
        "open_dm_conversation" => channel::open_dm_conversation(store, args).await,
        "list_dm_conversations" => channel::list_dm_conversations(store, args).await,
        "post_dm_message" => message::post_dm_message(server, args).await,
        "list_threads" => thread::list_threads(store, args).await,
        "list_mentions" => member::list_mentions(store, args).await,
        "get_inbox" => member::get_inbox(store, args).await,
        "mark_inbox_read" => member::mark_inbox_read(store, args).await,
        "list_messages" => message::list_messages(store, args).await,
        "post_message" => message::post_message(server, args).await,
        "edit_message" => message::edit_message(store, auth, args).await,
        "record_mention" => message::record_mention(store, args).await,
        "cast_vote" => social::cast_vote(store, args).await,
        "add_reaction" => social::add_reaction(store, args).await,
        "remove_reaction" => social::remove_reaction(store, args).await,
        "list_reactions" => social::list_reactions(store, args).await,
        "pin_message" => social::pin_message(store, args).await,
        "unpin_message" => social::unpin_message(store, args).await,
        "list_pins" => social::list_pins(store, args).await,
        "add_reference" => reference::add_reference(store, args).await,
        "upload_artifact" => artifact::upload_artifact(store, artifacts, args).await,
        "begin_artifact_multipart" => artifact::begin_artifact_multipart(artifacts).await,
        "upload_artifact_multipart_part" => {
            artifact::upload_artifact_multipart_part(artifacts, args).await
        }
        "complete_artifact_multipart" => {
            artifact::complete_artifact_multipart(store, artifacts, args).await
        }
        "abort_artifact_multipart" => artifact::abort_artifact_multipart(artifacts, args).await,
        "get_artifact_metadata" => artifact::get_artifact_metadata(store, args).await,
        "search_messages" => {
            search::search_messages(search, embedding_provider, store, auth, args).await
        }
        "register_slash_command" => automation::register_slash_command(store, auth, args).await,
        "list_slash_commands" => automation::list_slash_commands(store, auth, args).await,
        "register_fsm_hook" => automation::register_fsm_hook(store, auth, args).await,
        "list_fsm_hooks" => automation::list_fsm_hooks(store, auth, args).await,
        "get_thread_context" => {
            let v = crate::context::get_thread_context(store.as_ref(), args).await?;
            Ok(content_json(&v))
        }
        "get_workspace_context" => {
            let mut v = crate::context::get_workspace_context(store.as_ref(), args).await?;
            // Drop packed threads in private channels the caller can't access
            // (Cluster 162), caching the per-channel decision.
            if !auth.bypass {
                if let Some(threads) = v.get("threads").and_then(|t| t.as_array()) {
                    let mut decision: std::collections::HashMap<maidan_types::ChannelId, bool> =
                        std::collections::HashMap::new();
                    let mut kept = Vec::with_capacity(threads.len());
                    for t in threads {
                        let cid = t
                            .get("channel_id")
                            .and_then(|c| c.as_str())
                            .and_then(|s| s.parse::<uuid::Uuid>().ok())
                            .map(maidan_types::ChannelId);
                        let keep = match cid {
                            Some(id) => match decision.get(&id) {
                                Some(v) => *v,
                                None => {
                                    let ok =
                                        maidan_auth::can_access_channel(store.as_ref(), auth, id)
                                            .await?;
                                    decision.insert(id, ok);
                                    ok
                                }
                            },
                            None => true,
                        };
                        if keep {
                            kept.push(t.clone());
                        }
                    }
                    v["threads"] = Value::Array(kept);
                }
            }
            Ok(content_json(&v))
        }
        "summarize_thread" => thread::summarize_thread(server, session_id, args).await,
        other => Err(McpError::MethodNotFound(format!("tools/{other}"))),
    }
}

/// Wrap a JSON payload in MCP's `content[]` envelope. The MCP spec
/// requires tool results to be an array of content parts; for now we
/// always return a single `text` part with the JSON-stringified value.
pub(super) fn content_json<T: serde::Serialize>(value: &T) -> Value {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    json!({
        "content": [
            { "type": "text", "text": body }
        ],
        "isError": false
    })
}
