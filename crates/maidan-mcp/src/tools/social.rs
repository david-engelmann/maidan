//! Vote, reaction, and pin tool handlers.

use std::sync::Arc;

use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CastVoteArgs {
    message_id: uuid::Uuid,
    kind: VoteKind,
    #[serde(default)]
    confidence: Option<f64>,
}

pub(super) async fn cast_vote(
    server: &crate::server::McpServer,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CastVoteArgs = crate::tools::parse_args(args)?;
    if let Some(c) = a.confidence {
        if !(0.0..=1.0).contains(&c) {
            return Err(McpError::InvalidParams(
                "confidence must be in 0..=1".into(),
            ));
        }
    }
    // Emit the domain event (atomic) + bus-notify, like REST — so MCP
    // votes/reactions/pins reach WS/SSE, at-least-once, and federation.
    let stored = server
        .store
        .cast_vote_with_event(NewVote {
            message_id: MessageId(a.message_id),
            member_id: auth.member_id,
            kind: a.kind,
            confidence: a.confidence,
        })
        .await?;
    for event in &stored {
        server.publish_stored(event).await;
    }
    Ok(content_json(&json!({"ok": true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetractVoteArgs {
    message_id: uuid::Uuid,
    kind: VoteKind,
}

/// Take back the caller's own vote. Twin of `DELETE /messages/{id}/votes`.
pub(super) async fn retract_vote(
    server: &crate::server::McpServer,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RetractVoteArgs = crate::tools::parse_args(args)?;
    let (removed, stored) = server
        .store
        .retract_vote_with_event(MessageId(a.message_id), auth.member_id, a.kind)
        .await?;
    if let Some(stored) = stored {
        server.publish_stored(&stored).await;
    }
    Ok(content_json(&json!({"removed": removed})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReactionArgs {
    message_id: uuid::Uuid,
    emoji: String,
}

pub(super) async fn add_reaction(
    server: &crate::server::McpServer,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ReactionArgs = crate::tools::parse_args(args)?;
    let stored = server
        .store
        .add_reaction_with_event(NewReaction {
            message_id: MessageId(a.message_id),
            member_id: auth.member_id,
            emoji: a.emoji,
        })
        .await?;
    server.publish_stored(&stored).await;
    Ok(content_json(&json!({"ok": true})))
}

pub(super) async fn remove_reaction(
    server: &crate::server::McpServer,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ReactionArgs = crate::tools::parse_args(args)?;
    // The event is appended only when a row was actually removed (idempotent).
    let (removed, stored) = server
        .store
        .remove_reaction_with_event(MessageId(a.message_id), auth.member_id, &a.emoji)
        .await?;
    if let Some(stored) = stored {
        server.publish_stored(&stored).await;
    }
    Ok(content_json(&json!({"removed": removed})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListReactionsArgs {
    message_id: uuid::Uuid,
}

pub(super) async fn list_reactions(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListReactionsArgs = crate::tools::parse_args(args)?;
    let list = store
        .list_reactions_for_message(MessageId(a.message_id))
        .await?;
    Ok(content_json(&list))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinArgs {
    thread_id: uuid::Uuid,
    message_id: uuid::Uuid,
}

pub(super) async fn pin_message(
    server: &crate::server::McpServer,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: PinArgs = crate::tools::parse_args(args)?;
    let stored = server
        .store
        .pin_message_with_event(NewPin {
            thread_id: ThreadId(a.thread_id),
            message_id: MessageId(a.message_id),
            member_id: auth.member_id,
        })
        .await?;
    server.publish_stored(&stored).await;
    Ok(content_json(&json!({"ok": true})))
}

pub(super) async fn unpin_message(
    server: &crate::server::McpServer,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: PinArgs = crate::tools::parse_args(args)?;
    let (removed, stored) = server
        .store
        .unpin_message_with_event(
            ThreadId(a.thread_id),
            MessageId(a.message_id),
            auth.member_id,
        )
        .await?;
    if let Some(stored) = stored {
        server.publish_stored(&stored).await;
    }
    Ok(content_json(&json!({"removed": removed})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListPinsArgs {
    thread_id: uuid::Uuid,
}

pub(super) async fn list_pins(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListPinsArgs = crate::tools::parse_args(args)?;
    let list = store.list_pins_for_thread(ThreadId(a.thread_id)).await?;
    Ok(content_json(&list))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListVotesArgs {
    message_id: uuid::Uuid,
}

/// Votes on a message. Twin of `GET /messages/{id}/votes`.
pub(super) async fn list_votes(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListVotesArgs = crate::tools::parse_args(args)?;
    let votes = store
        .list_votes_for_message(MessageId(a.message_id))
        .await?;
    Ok(content_json(&votes))
}
