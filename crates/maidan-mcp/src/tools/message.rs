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

use maidan_auth::capability::MESSAGE_POST;
use maidan_auth::AuthContext;

use super::content_json;
use crate::error::McpError;

/// Pass a post's result through, first recording a `ThreadSpawnDenied` event
/// when the `max_tools` spawn-budget axis refused it — the MCP twin of the REST
/// `routes::observe_spawn_denial`. `actor` is the post's author: the store's
/// gate reports the thread and the numbers, not who pushed past the cap.
/// Best-effort, like every other MCP-published event.
pub(super) async fn observe_spawn_denial<T>(
    server: &crate::server::McpServer,
    actor: Option<MemberId>,
    result: Result<T, maidan_store::StoreError>,
) -> Result<T, McpError> {
    let err = match result {
        Ok(value) => return Ok(value),
        Err(err) => err,
    };
    if let maidan_store::StoreError::SpawnRejected(denial) = &err {
        server.publish_event(denial.denied_event(actor)).await;
    }
    Err(err.into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PostDmMessageArgs {
    dm_conversation_id: uuid::Uuid,
    #[serde(default)]
    body: String,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    content: Option<Vec<ContentBlock>>,
}

pub(super) async fn post_dm_message(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let a: PostDmMessageArgs = crate::tools::parse_args(args)?;
    let dm = store
        .get_dm_conversation(DmConversationId(a.dm_conversation_id))
        .await?;
    if dm.member_low_id != auth.member_id && dm.member_high_id != auth.member_id {
        return Err(McpError::InvalidParams(
            "authenticated member must be a DM participant".into(),
        ));
    }
    let content = a.content.clone();
    // Same rule as a channel post: an empty body is derived from typed
    // content, and omitting both is not a message.
    let body = body_from_content(&a.body, content.as_deref())?;
    let msg = store
        .post_message(NewMessage {
            thread_id: dm.thread_id,
            author_id: auth.member_id,
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
                sealed: None,
            })
            .await;
    }
    // Record + publish MentionRecorded per @mention so the notification router
    // / wait_for_mention fire (was recorded but never published).
    publish_routed_mentions(server, dm.thread_id, dm.workspace_id, &msg).await;
    // A human DM reply supersedes the agent's self-reported status, mirroring
    // the channel post path. Best-effort: the message is already posted.
    if let Ok(member) = store.get_member_in(dm.workspace_id, auth.member_id).await {
        if member.kind == MemberKind::Human {
            if let Err(e) = store.clear_thread_status(dm.thread_id).await {
                tracing::warn!(
                    thread_id = %dm.thread_id.0,
                    "clearing thread status after human DM post failed: {e}"
                );
            }
        }
    }
    Ok(content_json(&msg))
}

/// Route + record @mentions in a just-posted message and publish a
/// `MentionRecorded` event per mentioned member — the MCP analogue of the REST
/// `publish_routed_mentions`. Best-effort: a routing error is logged and
/// skipped, never failing the post.
pub(super) async fn publish_routed_mentions(
    server: &crate::server::McpServer,
    thread_id: ThreadId,
    workspace_id: WorkspaceId,
    message: &Message,
) {
    // Skip all store work when the body has no `@handles`, and route with the
    // workspace the caller already resolved (no per-post
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
#[serde(deny_unknown_fields)]
struct ListMessagesArgs {
    thread_id: uuid::Uuid,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

pub(super) async fn list_messages(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListMessagesArgs = crate::tools::parse_args(args)?;
    let messages = store
        .list_messages(ThreadId(a.thread_id), a.limit.clamp(1, 500))
        .await?;
    Ok(content_json(&messages))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PostMessageArgs {
    thread_id: uuid::Uuid,
    #[serde(default)]
    body: String,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    content: Option<Vec<ContentBlock>>,
}

/// Merge a slash-command's response metadata (`{slash_command,
/// slash_response}`) into the posted message's metadata — the maidan-mcp copy
/// of the REST `merge_metadata`, so an MCP slash post carries the same shape.
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

/// `body` is the searchable text. When it is omitted, derive it from typed
/// `content`. Omitting both is a client error: an empty post is not a content
/// post.
fn body_from_content(body: &str, content: Option<&[ContentBlock]>) -> Result<String, McpError> {
    if !body.is_empty() {
        return Ok(body.to_string());
    }
    match content {
        Some(blocks) if !blocks.is_empty() => Ok(derive_body(blocks)),
        _ => Err(McpError::InvalidParams(
            "body is required unless content is present".into(),
        )),
    }
}

pub(super) async fn post_message(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let a: PostMessageArgs = crate::tools::parse_args(args)?;
    let content = a.content.clone();
    let body = body_from_content(&a.body, content.as_deref())?;
    let thread_id = ThreadId(a.thread_id);
    let ctx = resolve_thread_context(store.as_ref(), thread_id)
        .await
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let new_message = NewMessage {
        thread_id,
        author_id: auth.member_id,
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

    // MCP posts now run registered slash commands, matching the REST post path.
    // The dispatcher is server-injected (attached only in the server binary);
    // without one — tests / embedders — this is the plain atomic post.
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

    // A post the `max_tools` axis refuses is recorded as `ThreadSpawnDenied` on
    // the way to the InvalidParams, on both branches.
    let author = Some(auth.member_id);
    let msg = if let Some((parsed, dispatcher)) = slash {
        // Provisional insert → run the (possibly external) dispatch →
        // finalizing edit + `MessagePosted` of the edited message in one tx.
        let provisional = store.post_message(new_message).await;
        let m = observe_spawn_denial(server, author, provisional).await?;
        let slash_meta = dispatcher
            .dispatch(
                auth,
                &parsed,
                ctx.workspace_id,
                ctx.channel_id,
                thread_id,
                auth.member_id,
                m.id,
            )
            .await;
        let metadata = merge_slash_metadata(m.metadata.clone(), slash_meta);
        let (message, stored) = store
            .edit_message_with_posted_event(
                m.id,
                auth.member_id,
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
        // The no-slash path is now the atomic outbox post
        // (`post_message_with_event` + `publish_stored`), matching REST — the
        // event is durably appended in the same tx (was a separate, bus-gated
        // append).
        let posted = store.post_message_with_event(new_message, dm_id).await;
        let (message, stored) = observe_spawn_denial(server, author, posted).await?;
        server.publish_stored(&stored).await;
        message
    };
    // Record + publish MentionRecorded per @mention (was recorded but never
    // published, so agent @mentions never fired the notification router /
    // wait_for_mention).
    publish_routed_mentions(server, thread_id, ctx.workspace_id, &msg).await;
    // A human response supersedes the agent's self-reported status, mirroring
    // the REST post path. Best-effort: the message is already posted.
    if let Ok(member) = store.get_member_in(ctx.workspace_id, auth.member_id).await {
        if member.kind == MemberKind::Human {
            if let Err(e) = store.clear_thread_status(thread_id).await {
                tracing::warn!(
                    thread_id = %thread_id.0,
                    "clearing thread status after human post failed: {e}"
                );
            }
        }
    }
    Ok(content_json(&msg))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditMessageArgs {
    message_id: uuid::Uuid,
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
    let a: EditMessageArgs = crate::tools::parse_args(args)?;
    let message_id = MessageId(a.message_id);
    let existing = store.get_message(message_id).await?;
    if existing.tombstoned_at.is_some() {
        return Err(McpError::InvalidParams("message is tombstoned".into()));
    }
    let editor_id = auth.member_id;
    // Author-only, as on REST: an edit keeps the author's name on the message,
    // so anyone else's edit would put words in their mouth.
    if !auth.bypass {
        if editor_id != existing.author_id {
            return Err(McpError::Forbidden(
                "only a message's author can edit it; another member's message can be tombstoned, not rewritten"
                    .into(),
            ));
        }
        maidan_auth::require_observed_capability(
            auth,
            maidan_auth::AuthorizationSurface::Mcp,
            MESSAGE_POST,
        )
        .map_err(McpError::from)?;
    }
    let metadata = match a.metadata {
        Some(v) if !v.is_null() => v,
        _ => existing.metadata,
    };
    // A content edit with no body re-derives the searchable body. Omitted
    // content keeps the existing blocks, but only once body or content was
    // actually sent — omitting both is the same client error as a post.
    let edit_body = body_from_content(&a.body, a.content.as_deref())?;
    let content = a.content.or(existing.content);
    // The edit + its `MessageEdited` event commit atomically, then the bus is
    // notified — so an MCP edit (like a REST edit) triggers embedding reindex,
    // feeds as-of context replay, and reaches WS/SSE subscribers. (MCP
    // previously called the event-less `edit_message`, silently breaking all
    // three.)
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
#[serde(deny_unknown_fields)]
struct RecordMentionArgs {
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

pub(super) async fn record_mention(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RecordMentionArgs = crate::tools::parse_args(args)?;
    // The explicit-mention API now emits MentionRecorded (atomic) + bus-notify,
    // so it reaches the notification router / wait_for_mention like REST.
    let stored = server
        .store
        .record_mention_with_event(MessageId(a.message_id), MemberId(a.member_id))
        .await?;
    server.publish_stored(&stored).await;
    Ok(content_json(&json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dm_post_derives_its_body_from_content_and_refuses_neither() {
        let with_content: PostDmMessageArgs = serde_json::from_value(json!({
            "dm_conversation_id": "00000000-0000-0000-0000-000000000001",
            "content": [{"type": "text", "text": "hello"}]
        }))
        .unwrap();
        assert_eq!(
            body_from_content(&with_content.body, with_content.content.as_deref()).unwrap(),
            "hello"
        );

        let neither: PostDmMessageArgs = serde_json::from_value(json!({
            "dm_conversation_id": "00000000-0000-0000-0000-000000000001"
        }))
        .unwrap();
        match body_from_content(&neither.body, neither.content.as_deref()) {
            Err(McpError::InvalidParams(msg)) => {
                assert!(
                    msg.contains("body is required unless content is present"),
                    "{msg}"
                );
            }
            other => panic!("expected invalid params, got {other:?}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TombstoneMessageArgs {
    message_id: uuid::Uuid,
}

/// Withdraw a message. Twin of `DELETE /messages/{id}`: `message:post`, and
/// `channel:admin` when the caller is not the author. Uses
/// `tombstone_message_with_event` plus the same DM conversation id REST attaches.
pub(super) async fn tombstone_message(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: TombstoneMessageArgs = crate::tools::parse_args(args)?;
    let message_id = MessageId(a.message_id);
    let chain = maidan_auth::authorize_message(server.store.as_ref(), auth, message_id).await?;
    let message = server.store.get_message(message_id).await?;
    if message.author_id != auth.member_id {
        maidan_auth::require_observed_capability(
            auth,
            maidan_auth::AuthorizationSurface::Mcp,
            maidan_auth::capability::CHANNEL_ADMIN,
        )
        .map_err(McpError::from)?;
    }
    let dm_conversation_id = server
        .store
        .dm_conversation_for_thread(chain.thread_id)
        .await?
        .map(|dm| dm.id);
    let stored = server
        .store
        .tombstone_message_with_event(message_id, dm_conversation_id)
        .await?;
    server.publish_stored(&stored).await;
    let uris =
        crate::resource_updates::uris_for_message_tombstone(server.store.as_ref(), message_id)
            .await;
    server.publish_resource_uris(uris).await;
    Ok(content_json(
        &json!({ "tombstoned": true, "message_id": a.message_id }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListMessageEditsArgs {
    message_id: uuid::Uuid,
    #[serde(default = "default_limit")]
    limit: i64,
}

/// Edit history of a message. Twin of `GET /messages/{id}/edits`. A tombstoned
/// message returns an empty history unless auth is bypassed, matching REST.
pub(super) async fn list_message_edits(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListMessageEditsArgs = crate::tools::parse_args(args)?;
    let message_id = MessageId(a.message_id);
    if !auth.bypass && store.get_message(message_id).await?.tombstoned_at.is_some() {
        return Ok(content_json(&Vec::<MessageEdit>::new()));
    }
    let edits = store
        .list_message_edits(message_id, a.limit.clamp(1, 500))
        .await?;
    Ok(content_json(&edits))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PostGroupDmMessageArgs {
    group_dm_conversation_id: uuid::Uuid,
    body: String,
    #[serde(default)]
    metadata: Value,
}

/// Post into a group DM. Twin of `POST /group-dms/{id}/messages`: `message:post`,
/// participant check, `post_message_with_event`, then mention routing.
pub(super) async fn post_group_dm_message(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: PostGroupDmMessageArgs = crate::tools::parse_args(args)?;
    let group_id = GroupDmConversationId(a.group_dm_conversation_id);
    let group = server.store.get_group_dm_conversation(group_id).await?;
    auth.ensure_workspace(group.workspace_id)
        .map_err(McpError::from)?;
    if !server
        .store
        .group_dm_has_member(group_id, auth.member_id)
        .await?
    {
        return Err(McpError::Forbidden(
            "member is not a participant in this group DM".into(),
        ));
    }
    let metadata = if a.metadata.is_null() {
        json!({})
    } else {
        a.metadata
    };
    let (msg, stored) = server
        .store
        .post_message_with_event(
            NewMessage {
                thread_id: group.thread_id,
                author_id: auth.member_id,
                body: a.body,
                metadata,
                content: None,
            },
            None,
        )
        .await?;
    server.publish_stored(&stored).await;
    publish_routed_mentions(server, group.thread_id, group.workspace_id, &msg).await;
    // A human group-DM reply supersedes the agent's self-reported status,
    // mirroring the channel post path. Best-effort: the message is already posted.
    if let Ok(member) = server
        .store
        .get_member_in(group.workspace_id, auth.member_id)
        .await
    {
        if member.kind == MemberKind::Human {
            if let Err(e) = server.store.clear_thread_status(group.thread_id).await {
                tracing::warn!(
                    thread_id = %group.thread_id.0,
                    "clearing thread status after human group DM post failed: {e}"
                );
            }
        }
    }
    Ok(content_json(&msg))
}
