//! Shared helpers for live event delivery (WebSocket, MCP SSE).

use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

use futures::StreamExt;
use maidan_bus::BusItem;
use maidan_store::Store;
use maidan_types::{BusEnvelope, Event, EventFilter, StoredEvent};
use serde::Serialize;
use tokio::sync::mpsc;

pub const REPLAY_LIMIT: i64 = 500;

#[derive(Debug, Serialize)]
pub struct ReplayHint {
    #[serde(rename = "type")]
    pub frame_type: &'static str,
    pub skipped: u64,
    pub after_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay: Option<String>,
}

pub fn envelope_from_stored(stored: &StoredEvent) -> Result<BusEnvelope, serde_json::Error> {
    let event: Event = serde_json::from_value(stored.payload.clone())?;
    Ok(BusEnvelope {
        log_id: stored.id,
        event,
    })
}

/// Replay persisted events matching `filter` with `id > after_id`. Returns the
/// highest `log_id` sent (still `after_id` if nothing matched).
pub async fn replay_matching_events(
    store: &dyn Store,
    filter: &EventFilter,
    after_id: i64,
    tx: &mpsc::Sender<String>,
) -> Result<i64, maidan_store::StoreError> {
    let Some(workspace_id) = filter.workspace_id else {
        return Ok(after_id);
    };
    let rows = store
        .list_events_after(workspace_id, after_id, REPLAY_LIMIT)
        .await?;
    let mut high_water = after_id;
    for row in rows {
        let envelope = match envelope_from_stored(&row) {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = %err, event_id = row.id, "drop stored event with invalid payload");
                continue;
            }
        };
        if !filter.matches_envelope(&envelope) {
            continue;
        }
        let payload = match serde_json::to_string(&envelope) {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(error = %err, "drop unserializable replay event");
                continue;
            }
        };
        if tx.send(payload).await.is_err() {
            break;
        }
        high_water = high_water.max(envelope.log_id);
    }
    Ok(high_water)
}

#[derive(Debug, Serialize)]
pub struct SubscribeAck {
    #[serde(rename = "type")]
    pub frame_type: &'static str,
    pub resume_token: String,
    pub after_id: i64,
}

pub fn subscribe_ack_payload(resume_token: &str, after_id: i64) -> Option<String> {
    let ack = SubscribeAck {
        frame_type: "subscribe_ack",
        resume_token: resume_token.to_string(),
        after_id,
    };
    serde_json::to_string(&ack).ok()
}

pub fn replay_hint_payload(
    skipped: u64,
    after_id: i64,
    replay_workspace: Option<maidan_types::WorkspaceId>,
) -> Option<String> {
    let hint = ReplayHint {
        frame_type: "replay_hint",
        skipped,
        after_id,
        workspace_id: replay_workspace.map(|w| w.0),
        replay: replay_workspace
            .map(|w| format!("/workspaces/{}/events?after_id={after_id}&limit=100", w.0)),
    };
    serde_json::to_string(&hint).ok()
}

/// Forward bus items with `log_id` strictly greater than `watermark`.
///
/// On [`BusItem::Lagged`], when `filter.workspace_id` is set the server replays
/// matching rows from `maidan_events` up to [`REPLAY_LIMIT`]. Otherwise it emits
/// a `replay_hint` frame for manual HTTP replay.
pub async fn forward_bus_items(
    mut subscriber: maidan_bus::EventStream,
    tx: mpsc::Sender<String>,
    watermark: Arc<AtomicI64>,
    store: Arc<dyn Store>,
    filter: EventFilter,
) {
    while let Some(item) = subscriber.next().await {
        match item {
            BusItem::Event(envelope) => {
                let id = envelope.log_id;
                if id <= watermark.load(Ordering::Relaxed) {
                    continue;
                }
                watermark.fetch_max(id, Ordering::Relaxed);
                let payload = match serde_json::to_string(envelope.as_ref()) {
                    Ok(p) => p,
                    Err(err) => {
                        tracing::warn!(error = %err, "drop unserializable event");
                        continue;
                    }
                };
                if tx.send(payload).await.is_err() {
                    break;
                }
            }
            BusItem::Lagged { skipped } => {
                let after_id = watermark.load(Ordering::Relaxed);
                if filter.workspace_id.is_some() {
                    match replay_matching_events(store.as_ref(), &filter, after_id, &tx).await {
                        Ok(hw) => {
                            watermark.fetch_max(hw, Ordering::Relaxed);
                            tracing::info!(
                                skipped,
                                after_id,
                                new_watermark = hw,
                                "auto-replayed events after bus lag"
                            );
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "auto-replay after bus lag failed");
                            if let Some(payload) =
                                replay_hint_payload(skipped, after_id, filter.workspace_id)
                            {
                                if tx.send(payload).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                } else if let Some(payload) = replay_hint_payload(skipped, after_id, None) {
                    if tx.send(payload).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}
