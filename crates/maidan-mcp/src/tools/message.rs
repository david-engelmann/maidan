//! Message posting, listing, editing, and mention-recording tool handlers.

use std::sync::Arc;

use chrono::Utc;
use maidan_router::{
    parse_at_handles, parse_slash_command, resolve_thread_context, route_mentions_in_message,
};
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_WRITE};
use maidan_auth::AuthContext;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct PostDmMessageArgs {
    dm_conversation_id: uuid::Uuid,
    author_id: uuid::Uuid,
    #[serde(default)]
    body: String,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    content: Option<Vec<ContentBlock>>,
}

pub(super) async fn post_dm_message(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let a: PostDmMessageArgs = serde_json::from_value(args.clone())?;
    let dm = store
        .get_dm_conversation(DmConversationId(a.dm_conversation_id))
        .await?;
    if dm.member_low_id != MemberId(a.author_id) && dm.member_high_id != MemberId(a.author_id) {
        return Err(McpError::InvalidParams(
            "author_id must be a DM participant".into(),
        ));
    }
    let content = a.content.clone();
    let body = if a.body.is_empty() {
        content.as_deref().map(derive_body).unwrap_or_default()
    } else {
        a.body.clone()
    };
    let msg = store
        .post_message(NewMessage {
            thread_id: dm.thread_id,
            author_id: MemberId(a.author_id),
            body,
            metadata: if a.metadata.is_null() {
                json!({})
            } else {
                a.metadata
            },
            content,
        })
        .await?;
    if server.event_bus.is_some() {
        let ctx = resolve_thread_context(store.as_ref(), dm.thread_id)
            .await
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        server
            .publish_event(Event::MessagePosted {
                occurred_at: Utc::now(),
                workspace_id: dm.workspace_id,
                channel_id: ctx.channel_id,
                thread_id: dm.thread_id,
                dm_conversation_id: Some(dm.id),
                message: msg.clone(),
            })
            .await;
    }
    // Cluster 334: record + publish MentionRecorded per @mention so the
    // notification router / wait_for_mention fire (was recorded but never published).
    publish_routed_mentions(server, dm.thread_id, dm.workspace_id, &msg).await;
    Ok(content_json(&msg))
}

/// Route + record @mentions in a just-posted message and publish a
/// `MentionRecorded` event per mentioned member — the MCP analogue of the REST
/// `publish_routed_mentions` (Cluster 334). Best-effort: a routing error is logged
/// and skipped, never failing the post.
async fn publish_routed_mentions(
    server: &crate::server::McpServer,
    thread_id: ThreadId,
    workspace_id: WorkspaceId,
    message: &Message,
) {
    // Cluster 338: skip all store work when the body has no `@handles`, and route
    // with the workspace the caller already resolved (no per-post
    // `resolve_message_chain` round-trip) — the parity of the REST change.
    if parse_at_handles(&message.body).is_empty() {
        return;
    }
    let mentioned = match route_mentions_in_message(
        server.store.as_ref(),
        workspace_id,
        message.id,
        message.author_id,
        &message.body,
    )
    .await
    {
        Ok(ids) => ids,
        Err(err) => {
            tracing::warn!(error = %err, "mcp mention routing failed");
            return;
        }
    };
    for member_id in mentioned {
        server
            .publish_event(Event::MentionRecorded {
                occurred_at: Utc::now(),
                workspace_id,
                thread_id,
                message_id: message.id,
                member_id,
            })
            .await;
    }
}

#[derive(Deserialize)]
struct ListMessagesArgs {
    thread_id: uuid::Uuid,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

pub(super) async fn list_messages(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListMessagesArgs = serde_json::from_value(args.clone())?;
    let messages = store
        .list_messages(ThreadId(a.thread_id), a.limit.clamp(1, 500))
        .await?;
    Ok(content_json(&messages))
}

#[derive(Deserialize)]
struct PostMessageArgs {
    thread_id: uuid::Uuid,
    author_id: uuid::Uuid,
    #[serde(default)]
    body: String,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    content: Option<Vec<ContentBlock>>,
}

/// Merge a slash-command's response metadata (`{slash_command, slash_response}`)
/// into the posted message's metadata — the maidan-mcp copy of the REST
/// `merge_metadata` (Cluster 345), so an MCP slash post carries the same shape.
fn merge_slash_metadata(mut base: Value, extra: Value) -> Value {
    if !base.is_object() {
        base = json!({});
    }
    if let (Some(base_obj), Some(extra_obj)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra_obj {
            base_obj.insert(key.clone(), value.clone());
        }
    }
    base
}

pub(super) async fn post_message(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let a: PostMessageArgs = serde_json::from_value(args.clone())?;
    let content = a.content.clone();
    let body = if a.body.is_empty() {
        content.as_deref().map(derive_body).unwrap_or_default()
    } else {
        a.body.clone()
    };
    let thread_id = ThreadId(a.thread_id);
    let ctx = resolve_thread_context(store.as_ref(), thread_id)
        .await
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let new_message = NewMessage {
        thread_id,
        author_id: MemberId(a.author_id),
        body,
        metadata: if a.metadata.is_null() {
            json!({})
        } else {
            a.metadata.clone()
        },
        content,
    };
    let dm_id = store
        .dm_conversation_for_thread(thread_id)
        .await
        .ok()
        .flatten()
        .map(|d| d.id);

    // Cluster 345: MCP posts now run registered slash commands, matching the REST
    // post path. The dispatcher is server-injected (attached only in the server
    // binary); without one — tests / embedders — this is the plain atomic post.
    let slash = match (
        parse_slash_command(&new_message.body),
        server.slash_dispatcher(),
    ) {
        (Some(parsed), Some(dispatcher))
            if store
                .get_slash_command_by_name(ctx.workspace_id, &parsed.name)
                .await
                .is_ok() =>
        {
            Some((parsed, dispatcher))
        }
        _ => None,
    };

    let msg = if let Some((parsed, dispatcher)) = slash {
        // Provisional insert → run the (possibly external) dispatch → finalizing
        // edit + `MessagePosted` of the edited message in one tx (Cluster 211 shape).
        let m = store.post_message(new_message).await?;
        let slash_meta = dispatcher
            .dispatch(
                auth,
                &parsed,
                ctx.workspace_id,
                ctx.channel_id,
                thread_id,
                MemberId(a.author_id),
                m.id,
            )
            .await;
        let metadata = merge_slash_metadata(m.metadata.clone(), slash_meta);
        let (message, stored) = store
            .edit_message_with_posted_event(
                m.id,
                MemberId(a.author_id),
                EditMessage {
                    body: m.body.clone(),
                    metadata,
                    content: m.content.clone(),
                },
                dm_id,
            )
            .await?;
        server.publish_stored(&stored).await;
        message
    } else {
        // Cluster 345: the no-slash path is now the atomic outbox post
        // (`post_message_with_event` + `publish_stored`), matching REST — the event
        // is durably appended in the same tx (was a separate, bus-gated append).
        let (message, stored) = store.post_message_with_event(new_message, dm_id).await?;
        server.publish_stored(&stored).await;
        message
    };
    // Cluster 334: record + publish MentionRecorded per @mention (was recorded but
    // never published, so agent @mentions never fired the notification router /
    // wait_for_mention).
    publish_routed_mentions(server, thread_id, ctx.workspace_id, &msg).await;
    Ok(content_json(&msg))
}

#[derive(Deserialize)]
struct EditMessageArgs {
    message_id: uuid::Uuid,
    editor_id: uuid::Uuid,
    #[serde(default)]
    body: String,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    content: Option<Vec<ContentBlock>>,
}

pub(super) async fn edit_message(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let a: EditMessageArgs = serde_json::from_value(args.clone())?;
    let message_id = MessageId(a.message_id);
    let existing = store.get_message(message_id).await?;
    if existing.tombstoned_at.is_some() {
        return Err(McpError::InvalidParams("message is tombstoned".into()));
    }
    let editor_id = MemberId(a.editor_id);
    if !auth.bypass {
        if editor_id == existing.author_id {
            auth.require_capability(MESSAGE_POST)
                .map_err(McpError::from)?;
        } else {
            auth.require_capability(WORKSPACE_WRITE)
                .map_err(McpError::from)?;
        }
    }
    let metadata = match a.metadata {
        Some(v) if !v.is_null() => v,
        _ => existing.metadata,
    };
    // Cluster 173: omitted content keeps existing; an empty body with content
    // re-derives the searchable body.
    let content = a.content.or(existing.content);
    let edit_body = if a.body.is_empty() {
        content.as_deref().map(derive_body).unwrap_or_default()
    } else {
        a.body
    };
    // Cluster 333: the edit + its `MessageEdited` event commit atomically, then
    // the bus is notified — so an MCP edit (like a REST edit) triggers embedding
    // reindex, feeds as-of context replay, and reaches WS/SSE subscribers. (MCP
    // previously called the event-less `edit_message`, silently breaking all three.)
    let dm_conversation_id = store
        .dm_conversation_for_thread(existing.thread_id)
        .await
        .ok()
        .flatten()
        .map(|d| d.id);
    let (msg, stored) = store
        .edit_message_with_event(
            message_id,
            editor_id,
            EditMessage {
                body: edit_body,
                metadata,
                content,
            },
            dm_conversation_id,
        )
        .await?;
    server.publish_stored(&stored).await;
    Ok(content_json(&msg))
}

#[derive(Deserialize)]
struct RecordMentionArgs {
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

pub(super) async fn record_mention(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RecordMentionArgs = serde_json::from_value(args.clone())?;
    // Cluster 334: the explicit-mention API now emits MentionRecorded (atomic) +
    // bus-notify, so it reaches the notification router / wait_for_mention like REST.
    let stored = server
        .store
        .record_mention_with_event(MessageId(a.message_id), MemberId(a.member_id))
        .await?;
    server.publish_stored(&stored).await;
    Ok(content_json(&json!({"ok": true})))
}
