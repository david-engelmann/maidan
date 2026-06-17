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
        | "get_workspace_context" => Ok(WORKSPACE_READ),
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

pub async fn dispatch(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    name: &str,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let artifacts = &server.artifacts;
    let search = &server.search;
    let embedding_provider = &server.embedding_provider;
    match name {
        "list_channels" => channel::list_channels(store, args).await,
        "open_dm_conversation" => channel::open_dm_conversation(store, args).await,
        "list_dm_conversations" => channel::list_dm_conversations(store, args).await,
        "post_dm_message" => message::post_dm_message(server, args).await,
        "list_threads" => thread::list_threads(store, args).await,
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
        "search_messages" => search::search_messages(search, embedding_provider, args).await,
        "register_slash_command" => automation::register_slash_command(store, auth, args).await,
        "list_slash_commands" => automation::list_slash_commands(store, auth, args).await,
        "register_fsm_hook" => automation::register_fsm_hook(store, auth, args).await,
        "list_fsm_hooks" => automation::list_fsm_hooks(store, auth, args).await,
        "get_thread_context" => {
            let v = crate::context::get_thread_context(store.as_ref(), args).await?;
            Ok(content_json(&v))
        }
        "get_workspace_context" => {
            let v = crate::context::get_workspace_context(store.as_ref(), args).await?;
            Ok(content_json(&v))
        }
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
