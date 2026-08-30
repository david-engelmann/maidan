//! Seed-from-message MCP tool (Cluster 328) — the twin of the REST route
//! (Cluster 327). Spawns a titled child thread from a source message + a
//! `seeded_from` reference edge; `inclusion=quote` also posts a first message
//! quoting the source. The source message's access is enforced by the
//! pre-dispatch gate (`message_id` arm); the target channel is checked here.

use maidan_auth::AuthContext;
use maidan_router::resolve_message_chain;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;
use crate::server::McpServer;

#[derive(Deserialize)]
struct SeedArgs {
    message_id: uuid::Uuid,
    title: String,
    #[serde(default)]
    inclusion: Option<String>,
    #[serde(default)]
    channel_id: Option<uuid::Uuid>,
}

pub(super) async fn seed_from_message(
    server: &McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let a: SeedArgs = serde_json::from_value(args.clone())?;
    let source_id = MessageId(a.message_id);
    let chain = resolve_message_chain(store.as_ref(), source_id)
        .await
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    if a.title.trim().is_empty() {
        return Err(McpError::InvalidParams("title must not be empty".into()));
    }
    let inclusion = a.inclusion.as_deref().unwrap_or("pointer");
    if !matches!(inclusion, "pointer" | "quote") {
        return Err(McpError::InvalidParams(
            "inclusion must be 'pointer' or 'quote'".into(),
        ));
    }
    // The source message's access is gated pre-dispatch; the target channel (which
    // may differ from the source's) is checked here.
    let target_channel = a.channel_id.map(ChannelId).unwrap_or(chain.channel_id);
    if !auth.bypass {
        maidan_auth::ensure_channel_access(store.as_ref(), auth, target_channel).await?;
    }

    // 1. The titled child thread (atomic row + ThreadCreated event).
    let (thread, t_stored) = store
        .create_thread_with_event(NewThread {
            channel_id: target_channel,
            parent_thread_id: None,
            title: Some(a.title.trim().to_string()),
        })
        .await?;
    notify(server, &t_stored).await;

    // 2. The lineage edge: new thread `seeded_from` the source message.
    let (_reference, r_stored) = store
        .add_reference_with_event(NewReference {
            src_kind: RefSide::Thread,
            src_id: thread.id.0,
            dst_kind: RefSide::Message,
            dst_id: a.message_id,
            relation: RelationKind::SeededFrom,
        })
        .await?;
    notify(server, &r_stored).await;

    // 3. `quote` inclusion: a first message quoting the source (author = caller).
    if inclusion == "quote" {
        let source = store.get_message(source_id).await?;
        let quoted = source
            .body
            .lines()
            .map(|l| format!("> {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (_message, m_stored) = store
            .post_message_with_event(
                NewMessage {
                    thread_id: thread.id,
                    author_id: auth.member_id,
                    body: quoted,
                    metadata: serde_json::json!({ "seeded_from": a.message_id }),
                    content: None,
                },
                None,
            )
            .await?;
        notify(server, &m_stored).await;
    }

    Ok(content_json(&thread))
}

/// Notify the bus of an already-durably-appended event (from a `*_with_event`
/// store call), hydrating the `Event` from the stored payload — the atomic
/// analogue of the REST `publish_stored`. A missing bus (embedded use) is a no-op.
async fn notify(server: &McpServer, stored: &StoredEvent) {
    if let Some(bus) = server.event_bus.as_ref() {
        if let Ok(event) = serde_json::from_value::<Event>(stored.payload.clone()) {
            let _ = bus
                .publish(BusEnvelope {
                    log_id: stored.id,
                    event,
                })
                .await;
        }
    }
}
