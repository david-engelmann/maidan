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
}

pub(super) async fn cast_vote(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: CastVoteArgs = serde_json::from_value(args.clone())?;
    store
        .cast_vote(NewVote {
            message_id: MessageId(a.message_id),
            member_id: MemberId(a.member_id),
            kind: a.kind,
        })
        .await?;
    Ok(content_json(&json!({"ok": true})))
}

#[derive(Deserialize)]
struct ReactionArgs {
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
    emoji: String,
}

pub(super) async fn add_reaction(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ReactionArgs = serde_json::from_value(args.clone())?;
    store
        .add_reaction(NewReaction {
            message_id: MessageId(a.message_id),
            member_id: MemberId(a.member_id),
            emoji: a.emoji,
        })
        .await?;
    Ok(content_json(&json!({"ok": true})))
}

pub(super) async fn remove_reaction(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ReactionArgs = serde_json::from_value(args.clone())?;
    let removed = store
        .remove_reaction(MessageId(a.message_id), MemberId(a.member_id), &a.emoji)
        .await?;
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

pub(super) async fn pin_message(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: PinArgs = serde_json::from_value(args.clone())?;
    store
        .pin_message(NewPin {
            thread_id: ThreadId(a.thread_id),
            message_id: MessageId(a.message_id),
            member_id: MemberId(a.member_id),
        })
        .await?;
    Ok(content_json(&json!({"ok": true})))
}

pub(super) async fn unpin_message(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: PinArgs = serde_json::from_value(args.clone())?;
    let removed = store
        .unpin_message(ThreadId(a.thread_id), MessageId(a.message_id))
        .await?;
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
