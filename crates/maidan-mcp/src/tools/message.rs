//! Message posting, listing, editing, and mention-recording tool handlers.

use std::sync::Arc;

use chrono::Utc;
use maidan_router::{resolve_thread_context, route_mentions_for_message};
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
    body: String,
    #[serde(default)]
    metadata: Value,
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
    let body = a.body.clone();
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
        })
        .await?;
    let _ = route_mentions_for_message(store.as_ref(), msg.id, msg.author_id, &msg.body).await;
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
    Ok(content_json(&msg))
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
    let messages = store.list_messages(ThreadId(a.thread_id), a.limit).await?;
    Ok(content_json(&messages))
}

#[derive(Deserialize)]
struct PostMessageArgs {
    thread_id: uuid::Uuid,
    author_id: uuid::Uuid,
    body: String,
    #[serde(default)]
    metadata: Value,
}

pub(super) async fn post_message(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let a: PostMessageArgs = serde_json::from_value(args.clone())?;
    let body = a.body.clone();
    let thread_id = ThreadId(a.thread_id);
    let msg = store
        .post_message(NewMessage {
            thread_id,
            author_id: MemberId(a.author_id),
            body,
            metadata: if a.metadata.is_null() {
                json!({})
            } else {
                a.metadata
            },
        })
        .await?;
    let _ = route_mentions_for_message(store.as_ref(), msg.id, msg.author_id, &msg.body).await;
    if server.event_bus.is_some() {
        let ctx = resolve_thread_context(store.as_ref(), thread_id)
            .await
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        let dm_id = store
            .dm_conversation_for_thread(thread_id)
            .await
            .ok()
            .flatten()
            .map(|d| d.id);
        server
            .publish_event(Event::MessagePosted {
                occurred_at: Utc::now(),
                workspace_id: ctx.workspace_id,
                channel_id: ctx.channel_id,
                thread_id,
                dm_conversation_id: dm_id,
                message: msg.clone(),
            })
            .await;
    }
    Ok(content_json(&msg))
}

#[derive(Deserialize)]
struct EditMessageArgs {
    message_id: uuid::Uuid,
    editor_id: uuid::Uuid,
    body: String,
    #[serde(default)]
    metadata: Option<Value>,
}

pub(super) async fn edit_message(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
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
    let msg = store
        .edit_message(
            message_id,
            editor_id,
            EditMessage {
                body: a.body,
                metadata,
            },
        )
        .await?;
    Ok(content_json(&msg))
}

#[derive(Deserialize)]
struct RecordMentionArgs {
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

pub(super) async fn record_mention(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RecordMentionArgs = serde_json::from_value(args.clone())?;
    store
        .record_mention(MessageId(a.message_id), MemberId(a.member_id))
        .await?;
    Ok(content_json(&json!({"ok": true})))
}
