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

mod approval;
mod artifact;
mod automation;
mod catalog;
mod channel;
mod member;
mod message;
mod reference;
mod roots;
mod schedule;
mod search;
mod skill;
mod social;
mod thread;

pub use catalog::catalog;

/// The tool catalog filtered to the tools the caller may invoke (Cluster 176,
/// token round 3). Bypass callers (auth disabled) see everything; otherwise a
/// tool whose required capability the caller lacks is omitted, so a
/// capability-scoped agent gets a smaller, relevant `tools/list` — fewer tokens
/// and no tools it would only get 403s from. The unfiltered [`catalog`] is
/// unchanged (contract tests + full-capability callers rely on it).
pub fn catalog_for(auth: &AuthContext) -> Vec<Value> {
    catalog()
        .into_iter()
        .filter(|tool| {
            if auth.bypass {
                return true;
            }
            tool.get("name")
                .and_then(|n| n.as_str())
                .and_then(|name| required_capability(name).ok())
                .is_some_and(|cap| auth.has_capability(cap))
        })
        .collect()
}

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
        | "get_tool_transcript"
        | "get_workspace_context"
        | "summarize_thread"
        | "request_approval"
        | "list_mentions"
        | "get_inbox"
        | "mark_inbox_read"
        | "wait_for_mention"
        | "wait_for_ready"
        | "get_queue_depth"
        | "list_assigned_threads"
        | "list_thread_dependencies"
        | "list_task_schedules"
        | "list_member_skills"
        | "list_thread_required_skills"
        | "list_roots" => Ok(WORKSPACE_READ),
        "open_dm_conversation" | "post_dm_message" | "post_message" | "edit_message" => {
            Ok(MESSAGE_POST)
        }
        "record_mention"
        | "cast_vote"
        | "add_reaction"
        | "remove_reaction"
        | "pin_message"
        | "unpin_message"
        | "add_reference"
        | "create_task_schedule"
        | "add_member_skill" => Ok(WORKSPACE_WRITE),
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
        "add_channel_member" | "list_channel_members" | "remove_channel_member" => {
            Ok(maidan_auth::capability::CHANNEL_ADMIN)
        }
        "assign_thread"
        | "claim_thread"
        | "unassign_thread"
        | "claim_next_thread"
        | "renew_claim"
        | "add_thread_dependency"
        | "add_thread_required_skill" => Ok(maidan_auth::capability::THREAD_TRANSITION),
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
        "list_threads"
        | "claim_next_thread"
        | "wait_for_ready"
        | "get_queue_depth"
        | "create_task_schedule" => {
            // `wait_for_ready`'s channel_id is optional; gate it only when present
            // so a caller can't long-poll a private channel they can't access.
            if let Some(id) = field("channel_id") {
                maidan_auth::ensure_channel_access(store, auth, maidan_types::ChannelId(id))
                    .await?;
            }
        }
        "list_messages"
        | "post_message"
        | "get_thread_context"
        | "get_tool_transcript"
        | "summarize_thread"
        | "pin_message"
        | "unpin_message"
        | "list_pins"
        | "assign_thread"
        | "claim_thread"
        | "unassign_thread"
        | "renew_claim"
        | "add_thread_dependency"
        | "list_thread_dependencies"
        | "add_thread_required_skill"
        | "list_thread_required_skills" => {
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
        "add_reference" => {
            for (kind_key, id_key) in [("src_kind", "src_id"), ("dst_kind", "dst_id")] {
                if let (Some(kv), Some(id)) = (args.get(kind_key), field(id_key)) {
                    match serde_json::from_value::<maidan_types::RefSide>(kv.clone()) {
                        Ok(maidan_types::RefSide::Thread) => {
                            maidan_auth::ensure_thread_access(
                                store,
                                auth,
                                maidan_types::ThreadId(id),
                            )
                            .await?;
                        }
                        Ok(maidan_types::RefSide::Message) => {
                            maidan_auth::ensure_message_access(
                                store,
                                auth,
                                maidan_types::MessageId(id),
                            )
                            .await?;
                        }
                        Err(_) => {}
                    }
                }
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
        "add_channel_member" => channel::add_channel_member(store, args).await,
        "list_channel_members" => channel::list_channel_members(store, args).await,
        "remove_channel_member" => channel::remove_channel_member(store, args).await,
        "open_dm_conversation" => channel::open_dm_conversation(store, args).await,
        "list_dm_conversations" => channel::list_dm_conversations(store, args).await,
        "post_dm_message" => message::post_dm_message(server, args).await,
        "list_threads" => thread::list_threads(store, args).await,
        "get_tool_transcript" => thread::get_tool_transcript(store, args).await,
        "assign_thread" => thread::assign_thread(server, args).await,
        "claim_thread" => thread::claim_thread(server, args).await,
        "unassign_thread" => thread::unassign_thread(server, args).await,
        "list_assigned_threads" => thread::list_assigned_threads(store, auth, args).await,
        "claim_next_thread" => thread::claim_next_thread(server, args).await,
        "renew_claim" => thread::renew_claim(server, args).await,
        "add_thread_dependency" => thread::add_thread_dependency(store, auth, args).await,
        "list_thread_dependencies" => thread::list_thread_dependencies(store, args).await,
        "list_mentions" => member::list_mentions(store, args).await,
        "get_inbox" => member::get_inbox(store, args).await,
        "mark_inbox_read" => member::mark_inbox_read(store, args).await,
        "wait_for_mention" => member::wait_for_mention(server, auth, args).await,
        "wait_for_ready" => thread::wait_for_ready(server, auth, args).await,
        "get_queue_depth" => thread::get_queue_depth(store, args).await,
        "create_task_schedule" => schedule::create_task_schedule(store, auth, args).await,
        "list_task_schedules" => schedule::list_task_schedules(store, auth, args).await,
        "add_member_skill" => skill::add_member_skill(store, args).await,
        "list_member_skills" => skill::list_member_skills(store, args).await,
        "add_thread_required_skill" => skill::add_thread_required_skill(store, args).await,
        "list_thread_required_skills" => skill::list_thread_required_skills(store, args).await,
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
            // Thread-keyed + DM-participant-aware (Cluster 180; channel-keyed
            // exempted `__dm__` and leaked DM threads into the context pack).
            if !auth.bypass {
                if let Some(threads) = v.get("threads").and_then(|t| t.as_array()) {
                    let mut decision: std::collections::HashMap<maidan_types::ThreadId, bool> =
                        std::collections::HashMap::new();
                    let mut kept = Vec::with_capacity(threads.len());
                    for t in threads {
                        let tid = t
                            .get("thread")
                            .and_then(|th| th.get("id"))
                            .and_then(|c| c.as_str())
                            .and_then(|s| s.parse::<uuid::Uuid>().ok())
                            .map(maidan_types::ThreadId);
                        let keep = match tid {
                            Some(id) => match decision.get(&id) {
                                Some(v) => *v,
                                None => {
                                    let ok =
                                        maidan_auth::can_access_thread(store.as_ref(), auth, id)
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
        "request_approval" => approval::request_approval(server, session_id, args).await,
        "list_roots" => roots::list_roots(server, session_id, args).await,
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

#[cfg(test)]
mod catalog_filter_tests {
    use super::*;
    use maidan_types::{ApiTokenId, MemberId, WorkspaceId};

    fn tool_names(tools: &[Value]) -> Vec<String> {
        tools
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect()
    }

    #[test]
    fn bypass_sees_the_whole_catalog() {
        let auth = AuthContext::bypass();
        assert_eq!(catalog_for(&auth).len(), catalog().len());
    }

    #[test]
    fn a_read_only_token_sees_only_read_tools() {
        let auth = AuthContext::from_token(
            ApiTokenId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
            WorkspaceId(uuid::Uuid::new_v4()),
            vec![WORKSPACE_READ.to_string()],
        );
        let names = tool_names(&catalog_for(&auth));
        // A workspace:read tool is present; write / search / artifact tools are not.
        assert!(names.contains(&"list_threads".to_string()));
        assert!(!names.contains(&"post_message".to_string())); // message:post
        assert!(!names.contains(&"search_messages".to_string())); // search:query
        assert!(!names.contains(&"add_reaction".to_string())); // workspace:write
                                                               // Every surfaced tool really does require workspace:read.
        for name in &names {
            assert_eq!(required_capability(name).unwrap(), WORKSPACE_READ, "{name}");
        }
    }

    #[test]
    fn a_missing_capability_hides_its_tools() {
        let auth = AuthContext::from_token(
            ApiTokenId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
            WorkspaceId(uuid::Uuid::new_v4()),
            vec![SEARCH_QUERY.to_string()],
        );
        let names = tool_names(&catalog_for(&auth));
        assert_eq!(names, vec!["search_messages".to_string()]);
    }
}
