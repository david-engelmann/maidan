//! Shared helpers for live event delivery (WebSocket, MCP SSE).

use std::sync::atomic::{AtomicI64, Ordering};

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
pub async fn forward_bus_items(
    mut subscriber: maidan_bus::EventStream,
    tx: mpsc::Sender<String>,
    watermark: std::sync::Arc<AtomicI64>,
    replay_workspace: Option<maidan_types::WorkspaceId>,
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
                let Some(payload) = replay_hint_payload(skipped, after_id, replay_workspace) else {
                    continue;
                };
                if tx.send(payload).await.is_err() {
                    break;
                }
            }
        }
    }
}
