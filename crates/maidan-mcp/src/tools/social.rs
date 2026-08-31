//! Vote, reaction, and pin tool handlers.

use std::sync::Arc;

use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct CastVoteArgs {
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
    kind: String,
    #[serde(default)]
    confidence: Option<f64>,
}

pub(super) async fn cast_vote(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CastVoteArgs = serde_json::from_value(args.clone())?;
    if let Some(c) = a.confidence {
        if !(0.0..=1.0).contains(&c) {
            return Err(McpError::InvalidParams(
                "confidence must be in 0..=1".into(),
            ));
        }
    }
    // Cluster 334: emit the domain event (atomic) + bus-notify, like REST — so
    // MCP votes/reactions/pins reach WS/SSE, at-least-once, and federation.
    let stored = server
        .store
        .cast_vote_with_event(NewVote {
            message_id: MessageId(a.message_id),
            member_id: MemberId(a.member_id),
            kind: a.kind,
            confidence: a.confidence,
        })
        .await?;
    server.publish_stored(&stored).await;
    Ok(content_json(&json!({"ok": true})))
}

#[derive(Deserialize)]
struct ReactionArgs {
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
    emoji: String,
}

pub(super) async fn add_reaction(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ReactionArgs = serde_json::from_value(args.clone())?;
    let stored = server
        .store
        .add_reaction_with_event(NewReaction {
            message_id: MessageId(a.message_id),
            member_id: MemberId(a.member_id),
            emoji: a.emoji,
        })
        .await?;
    server.publish_stored(&stored).await;
    Ok(content_json(&json!({"ok": true})))
}

pub(super) async fn remove_reaction(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ReactionArgs = serde_json::from_value(args.clone())?;
    // The event is appended only when a row was actually removed (idempotent).
    let (removed, stored) = server
        .store
        .remove_reaction_with_event(MessageId(a.message_id), MemberId(a.member_id), &a.emoji)
        .await?;
    if let Some(stored) = stored {
        server.publish_stored(&stored).await;
    }
    Ok(content_json(&json!({"removed": removed})))
}

#[derive(Deserialize)]
struct ListReactionsArgs {
    message_id: uuid::Uuid,
}

pub(super) async fn list_reactions(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListReactionsArgs = serde_json::from_value(args.clone())?;
    let list = store
        .list_reactions_for_message(MessageId(a.message_id))
        .await?;
    Ok(content_json(&list))
}

#[derive(Deserialize)]
struct PinArgs {
    thread_id: uuid::Uuid,
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

pub(super) async fn pin_message(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: PinArgs = serde_json::from_value(args.clone())?;
    let stored = server
        .store
        .pin_message_with_event(NewPin {
            thread_id: ThreadId(a.thread_id),
            message_id: MessageId(a.message_id),
            member_id: MemberId(a.member_id),
        })
        .await?;
    server.publish_stored(&stored).await;
    Ok(content_json(&json!({"ok": true})))
}

pub(super) async fn unpin_message(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: PinArgs = serde_json::from_value(args.clone())?;
    let (removed, stored) = server
        .store
        .unpin_message_with_event(
            ThreadId(a.thread_id),
            MessageId(a.message_id),
            MemberId(a.member_id),
        )
        .await?;
    if let Some(stored) = stored {
        server.publish_stored(&stored).await;
    }
    Ok(content_json(&json!({"removed": removed})))
}

#[derive(Deserialize)]
struct ListPinsArgs {
    thread_id: uuid::Uuid,
}

pub(super) async fn list_pins(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListPinsArgs = serde_json::from_value(args.clone())?;
    let list = store.list_pins_for_thread(ThreadId(a.thread_id)).await?;
    Ok(content_json(&list))
}
