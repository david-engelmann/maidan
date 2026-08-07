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

use crate::subscribe_metrics::{
    record_bus_lag, record_subscribe_replay, SubscribeReplayOutcome, SubscribeTransport,
};

pub const REPLAY_LIMIT: i64 = 500;

/// Coalesce the optimistic-path delivery-cursor write (Cluster 169, H2): buffer
/// the highest delivered `log_id` and persist it at most once per this many
/// events or [`CURSOR_FLUSH_INTERVAL`], plus a final flush when the stream ends.
/// The cursor is best-effort on this path (the authoritative at-least-once path
/// is [`reconcile_deliver`], which already batches), and `advance_delivery_cursor`
/// is monotonic, so a coalesced-away write only means an at-least-once reconnect
/// re-delivers a few already-seen events — the contract already tolerates that.
const CURSOR_FLUSH_EVENTS: u32 = 64;
const CURSOR_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Persist a buffered delivery cursor (Cluster 169, H2). No-op when nothing is
/// buffered or no consumer/workspace resolved.
async fn flush_delivery_cursor(
    store: &Arc<dyn Store>,
    writer: Option<(&str, maidan_types::WorkspaceId)>,
    pending: &mut Option<i64>,
) {
    if let (Some((consumer_id, workspace_id)), Some(log_id)) = (writer, pending.take()) {
        if let Err(err) = store
            .advance_delivery_cursor(consumer_id, workspace_id, log_id)
            .await
        {
            tracing::warn!(
                error = %err,
                consumer_id,
                log_id,
                "delivery cursor advance failed"
            );
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReplayOutcome {
    pub high_water: i64,
    /// `true` when the store returned exactly [`REPLAY_LIMIT`] rows (more may remain).
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ReplayTruncated {
    #[serde(rename = "type")]
    pub frame_type: &'static str,
    pub after_id: i64,
    pub limit: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<uuid::Uuid>,
}

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

pub fn replay_truncated_payload(
    after_id: i64,
    workspace_id: Option<maidan_types::WorkspaceId>,
) -> Option<String> {
    let frame = ReplayTruncated {
        frame_type: "replay_truncated",
        after_id,
        limit: REPLAY_LIMIT,
        workspace_id: workspace_id.map(|w| w.0),
    };
    serde_json::to_string(&frame).ok()
}

pub async fn emit_replay_truncated_if_needed(
    tx: &mpsc::Sender<String>,
    after_id: i64,
    workspace_id: Option<maidan_types::WorkspaceId>,
    truncated: bool,
) {
    if !truncated {
        return;
    }
    if let Some(payload) = replay_truncated_payload(after_id, workspace_id) {
        let _ = tx.send(payload).await;
    }
}

/// Replay persisted events matching `filter` with `id > after_id`.
pub async fn replay_matching_events(
    store: &dyn Store,
    filter: &EventFilter,
    after_id: i64,
    tx: &mpsc::Sender<String>,
    delivery_consumer_id: Option<&str>,
) -> Result<ReplayOutcome, maidan_store::StoreError> {
    let Some(workspace_id) = filter.workspace_id else {
        return Ok(ReplayOutcome {
            high_water: after_id,
            truncated: false,
        });
    };
    let rows = store
        .list_events_after(workspace_id, after_id, REPLAY_LIMIT)
        .await?;
    let truncated = rows.len() as i64 == REPLAY_LIMIT;
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
    // Coalesce the per-row cursor writes into one advance to the batch high-water
    // (Cluster 169, H2) — monotonic, so this is equivalent to the per-row writes.
    if high_water > after_id {
        if let (Some(consumer_id), Some(workspace_id)) = (delivery_consumer_id, filter.workspace_id)
        {
            let _ = store
                .advance_delivery_cursor(consumer_id, workspace_id, high_water)
                .await;
        }
    }
    Ok(ReplayOutcome {
        high_water,
        truncated,
    })
}

/// Wire protocol version for WebSocket / MCP SSE subscribe (Cluster 62).
pub const SUBSCRIBE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct SubscribeAck {
    #[serde(rename = "type")]
    pub frame_type: &'static str,
    pub schema_version: u32,
    pub resume_token: String,
    pub after_id: i64,
}

pub fn subscribe_ack_payload(resume_token: &str, after_id: i64) -> Option<String> {
    let ack = SubscribeAck {
        frame_type: "subscribe_ack",
        schema_version: SUBSCRIBE_SCHEMA_VERSION,
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
    transport: SubscribeTransport,
    delivery_consumer_id: Option<String>,
) {
    // Best-effort cursor on the optimistic path, coalesced (Cluster 169, H2):
    // buffer the highest delivered id and persist it on a count/time threshold
    // + a final flush, instead of a DB write per event.
    let cursor_writer = delivery_consumer_id.as_deref().zip(filter.workspace_id);
    let mut pending_cursor: Option<i64> = None;
    let mut events_since_flush: u32 = 0;
    let mut last_flush = std::time::Instant::now();
    while let Some(item) = subscriber.next().await {
        match item {
            BusItem::Event(envelope) => {
                let id = envelope.log_id;
                if id <= watermark.load(Ordering::Relaxed) {
                    continue;
                }
                watermark.fetch_max(id, Ordering::Relaxed);
                if cursor_writer.is_some() {
                    pending_cursor = Some(pending_cursor.map_or(id, |p| p.max(id)));
                    events_since_flush += 1;
                    if events_since_flush >= CURSOR_FLUSH_EVENTS
                        || last_flush.elapsed() >= CURSOR_FLUSH_INTERVAL
                    {
                        flush_delivery_cursor(&store, cursor_writer, &mut pending_cursor).await;
                        events_since_flush = 0;
                        last_flush = std::time::Instant::now();
                    }
                }
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
                record_bus_lag(transport, skipped);
                let after_id = watermark.load(Ordering::Relaxed);
                if filter.workspace_id.is_some() {
                    match replay_matching_events(
                        store.as_ref(),
                        &filter,
                        after_id,
                        &tx,
                        delivery_consumer_id.as_deref(),
                    )
                    .await
                    {
                        Ok(outcome) => {
                            watermark.fetch_max(outcome.high_water, Ordering::Relaxed);
                            record_subscribe_replay(transport, SubscribeReplayOutcome::AutoReplay);
                            emit_replay_truncated_if_needed(
                                &tx,
                                outcome.high_water,
                                filter.workspace_id,
                                outcome.truncated,
                            )
                            .await;
                            if outcome.truncated {
                                record_subscribe_replay(
                                    transport,
                                    SubscribeReplayOutcome::ReplayTruncated,
                                );
                            }
                            tracing::info!(
                                skipped,
                                after_id,
                                new_watermark = outcome.high_water,
                                truncated = outcome.truncated,
                                "auto-replayed events after bus lag"
                            );
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "auto-replay after bus lag failed");
                            record_subscribe_replay(
                                transport,
                                SubscribeReplayOutcome::AutoReplayFailed,
                            );
                            if let Some(payload) =
                                replay_hint_payload(skipped, after_id, filter.workspace_id)
                            {
                                if tx.send(payload).await.is_err() {
                                    break;
                                }
                                record_subscribe_replay(
                                    transport,
                                    SubscribeReplayOutcome::ReplayHint,
                                );
                            }
                        }
                    }
                } else if let Some(payload) = replay_hint_payload(skipped, after_id, None) {
                    if tx.send(payload).await.is_err() {
                        break;
                    }
                    record_subscribe_replay(transport, SubscribeReplayOutcome::ReplayHint);
                }
            }
        }
    }
    // Persist whatever's buffered when the stream ends (Cluster 169, H2).
    flush_delivery_cursor(&store, cursor_writer, &mut pending_cursor).await;
}

/// Stability window for at-least-once reconcile delivery (Cluster 125): a row is
/// eligible only once its `inserted_at` is older than this. Must exceed the
/// longest insert-transaction duration. Default 2s; `0` disables the gate.
pub fn reconcile_stability_window_from_env() -> std::time::Duration {
    std::env::var("MAIDAN_DELIVERY_STABILITY_SECS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|&s| s >= 0.0)
        .map(std::time::Duration::from_secs_f64)
        .unwrap_or_else(|| std::time::Duration::from_secs(2))
}

/// Poll cadence for the reconcile loop (a NOTIFY also wakes it). Default 1s.
pub fn reconcile_interval_from_env() -> std::time::Duration {
    std::env::var("MAIDAN_DELIVERY_RECONCILE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&ms| ms > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(1000))
}

/// Cursor-driven **at-least-once** delivery (Cluster 125).
///
/// Polls stable rows (`inserted_at <= now - stability`) with `id > cursor` from
/// the durable delivery cursor, in strict `id` order, delivers the matching ones
/// and advances the cursor. `wake` is the bus subscription, used only as a
/// low-latency hint (its contents are ignored). Because delivery reads from a
/// contiguous cursor and only stable rows, no committed event is ever skipped by
/// an out-of-order publish or a late-committing serial — the gap the optimistic
/// [`forward_bus_items`] path can drop. The cost is a stability-window latency
/// floor on fresh events; the backlog (already stable) is delivered immediately.
#[allow(clippy::too_many_arguments)]
pub async fn reconcile_deliver(
    mut wake: maidan_bus::EventStream,
    tx: mpsc::Sender<String>,
    store: Arc<dyn Store>,
    filter: EventFilter,
    workspace_id: maidan_types::WorkspaceId,
    consumer_id: String,
    start_after_id: i64,
    stability: std::time::Duration,
    poll_interval: std::time::Duration,
) {
    let mut cursor = start_after_id;
    let mut ticker = tokio::time::interval(poll_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        // Wake on either the poll tick or any bus notification (content ignored).
        tokio::select! {
            _ = ticker.tick() => {}
            item = wake.next() => {
                if item.is_none() {
                    break;
                }
            }
        }
        // Drain all currently-stable rows above the cursor, in id order.
        loop {
            let window =
                chrono::Duration::from_std(stability).unwrap_or_else(|_| chrono::Duration::zero());
            let cutoff = chrono::Utc::now() - window;
            let rows = match store
                .list_events_after_stable(workspace_id, cursor, cutoff, REPLAY_LIMIT)
                .await
            {
                Ok(rows) => rows,
                Err(err) => {
                    tracing::warn!(error = %err, "reconcile read failed");
                    break;
                }
            };
            if rows.is_empty() {
                break;
            }
            let batch_len = rows.len() as i64;
            let mut batch_max = cursor;
            for row in rows {
                // Advance past every examined row (matched or not) — the cursor
                // is this consumer's position in the workspace event stream.
                batch_max = batch_max.max(row.id);
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
                        tracing::warn!(error = %err, "drop unserializable reconcile event");
                        continue;
                    }
                };
                if tx.send(payload).await.is_err() {
                    return;
                }
            }
            cursor = batch_max;
            if let Err(err) = store
                .advance_delivery_cursor(&consumer_id, workspace_id, cursor)
                .await
            {
                tracing::warn!(error = %err, consumer_id, cursor, "reconcile cursor advance failed");
            }
            if batch_len < REPLAY_LIMIT {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_truncated_payload_includes_limit_and_watermark() {
        let payload = replay_truncated_payload(99, None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["type"], "replay_truncated");
        assert_eq!(v["after_id"], 99);
        assert_eq!(v["limit"], REPLAY_LIMIT);
    }
}
