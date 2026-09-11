//! The AG-UI SSE door (`GET /agui/stream`, Cluster 369.2, Wave 2 #17, H1).
//!
//! Reuses the event bus + resume machinery behind [`crate::mcp_stream`], but
//! emits [AG-UI](https://docs.ag-ui.com) protocol events (`RUN_STARTED`,
//! `TEXT_MESSAGE_*`, `TOOL_CALL_*`, `RUN_FINISHED`, …) instead of Maidan wire
//! frames — so an AG-UI-compatible frontend renders agent activity with no
//! adapter. A **thread is a run** (see [`crate::agui`] for the mapping).
//!
//! Resume: the SSE-standard `Last-Event-ID` header (or an `after_id` query
//! param) replays persisted events with a higher log id before live delivery,
//! and every emitted frame carries its source `id:` (the event-log id) so a
//! reconnecting client resumes exactly where it dropped. Each event is
//! RBAC-filtered per-recipient (`can_access_thread` / `can_access_channel`), so
//! a workspace-wide stream never leaks a private channel to a non-member.
//!
//! Output direction only: Maidan → AG-UI. The UI→agent input direction
//! (interrupts, `editedArgs` = full replace) is a follow-up; REST already
//! carries those mutations.

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    Extension,
};
use futures::StreamExt;
use maidan_auth::{capability::EVENT_SUBSCRIBE, AuthContext};
use maidan_bus::BusItem;
use maidan_store::Store;
use maidan_types::{ChannelId, Event, EventFilter, ThreadId, WorkspaceId};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::agui::agui_events_for;
use crate::error::ApiError;
use crate::event_stream::{envelope_from_stored, REPLAY_LIMIT};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct AgUiStreamQuery {
    #[serde(default)]
    pub workspace_id: Option<uuid::Uuid>,
    /// Narrow the run stream to one thread (a single run) or one channel.
    #[serde(default)]
    pub thread_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub channel_id: Option<uuid::Uuid>,
    /// Resume: replay persisted events with `id > after_id` before live
    /// delivery. The SSE-standard `Last-Event-ID` header takes precedence.
    #[serde(default)]
    pub after_id: Option<i64>,
}

pub async fn stream(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    Query(q): Query<AgUiStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    auth.require_capability(EVENT_SUBSCRIBE)
        .map_err(|_| ApiError::Forbidden("missing event:subscribe capability".into()))?;

    let mut filter = EventFilter::all();
    if let Some(ws) = q.workspace_id {
        filter.workspace_id = Some(WorkspaceId(ws));
        auth.ensure_workspace(WorkspaceId(ws))
            .map_err(|_| ApiError::Forbidden("token is not valid for this workspace".into()))?;
    }
    if let Some(ch) = q.channel_id {
        filter.channel_id = Some(ChannelId(ch));
    }
    if let Some(th) = q.thread_id {
        filter.thread_id = Some(ThreadId(th));
    }

    // `Last-Event-ID` (SSE reconnect) wins over the `after_id` query param.
    let after_id = last_event_id(&headers).or(q.after_id).unwrap_or(0);
    if after_id < 0 {
        return Err(ApiError::BadRequest("after_id must be non-negative".into()));
    }
    if after_id > 0 && filter.workspace_id.is_none() {
        return Err(ApiError::BadRequest(
            "after_id requires a workspace_id to replay".into(),
        ));
    }

    // Each frame carries `(log_id, agui_json)` so the SSE `id:` is the source
    // event-log id — the resume anchor a reconnect sends back as Last-Event-ID.
    let (sse_tx, sse_rx) = mpsc::channel::<Result<SseEvent, Infallible>>(256);

    let subscriber = state
        .bus
        .subscribe(filter.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let store = state.store.clone();
    tokio::spawn(async move {
        let mut high_water = after_id;
        if after_id > 0 {
            match replay_agui(store.as_ref(), &auth, &filter, after_id, &sse_tx).await {
                Ok(hw) => high_water = hw,
                Err(err) => {
                    tracing::warn!(error = %err, "agui: replay failed");
                }
            }
        }
        forward_agui(subscriber, store.as_ref(), &auth, high_water, &sse_tx).await;
    });

    let stream = ReceiverStream::new(sse_rx);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

/// The `Last-Event-ID` header as a non-negative event-log id, if present and
/// parseable (a malformed value is ignored — the query `after_id` still applies).
fn last_event_id(headers: &HeaderMap) -> Option<i64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|id| *id >= 0)
}

/// Replay persisted events with `id > after_id` as AG-UI frames. Returns the
/// high-water log id reached (so live forwarding can dedup against it).
async fn replay_agui(
    store: &dyn Store,
    auth: &AuthContext,
    filter: &EventFilter,
    after_id: i64,
    tx: &mpsc::Sender<Result<SseEvent, Infallible>>,
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
                tracing::warn!(error = %err, event_id = row.id, "agui: drop invalid replay event");
                continue;
            }
        };
        if !filter.matches_envelope(&envelope) {
            continue;
        }
        if !event_visible(store, auth, &envelope.event).await {
            continue;
        }
        if !send_agui(tx, envelope.log_id, &envelope.event).await {
            break;
        }
        high_water = high_water.max(envelope.log_id);
    }
    Ok(high_water)
}

/// Forward live bus items as AG-UI frames, skipping any `log_id <= high_water`
/// already covered by replay. A bus lag surfaces as a `CUSTOM` `lagged` event so
/// the UI can prompt a manual reconnect (Last-Event-ID resume).
async fn forward_agui(
    mut subscriber: maidan_bus::EventStream,
    store: &dyn Store,
    auth: &AuthContext,
    mut high_water: i64,
    tx: &mpsc::Sender<Result<SseEvent, Infallible>>,
) {
    while let Some(item) = subscriber.next().await {
        match item {
            BusItem::Event(envelope) => {
                if envelope.log_id <= high_water {
                    continue;
                }
                high_water = high_water.max(envelope.log_id);
                if !event_visible(store, auth, &envelope.event).await {
                    continue;
                }
                if !send_agui(tx, envelope.log_id, &envelope.event).await {
                    break;
                }
            }
            BusItem::Lagged { skipped } => {
                let lagged = crate::agui::AgUiEvent::Custom {
                    name: "lagged".into(),
                    value: serde_json::json!({ "skipped": skipped, "afterId": high_water }),
                };
                if let Ok(data) = serde_json::to_string(&lagged) {
                    if tx.send(Ok(SseEvent::default().data(data))).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

/// Whether this recipient may see the event: a thread-scoped event is gated by
/// `can_access_thread`, a channel-only one by `can_access_channel`, and a
/// workspace-level one (already workspace-scoped) is visible. A store error is
/// treated as not-visible (fail-closed) rather than leaking.
async fn event_visible(store: &dyn Store, auth: &AuthContext, event: &Event) -> bool {
    if let Some(thread_id) = event.thread_id() {
        return matches!(
            maidan_auth::can_access_thread(store, auth, thread_id).await,
            Ok(true)
        );
    }
    if let Some(channel_id) = event.channel_id() {
        return matches!(
            maidan_auth::can_access_channel(store, auth, channel_id).await,
            Ok(true)
        );
    }
    true
}

/// Map one Maidan event to its AG-UI frames and send them, each tagged with the
/// source `log_id` as the SSE `id:`. Returns `false` when the receiver is gone.
async fn send_agui(
    tx: &mpsc::Sender<Result<SseEvent, Infallible>>,
    log_id: i64,
    event: &Event,
) -> bool {
    for agui in agui_events_for(event) {
        let data = match serde_json::to_string(&agui) {
            Ok(d) => d,
            Err(err) => {
                tracing::warn!(error = %err, "agui: drop unserializable frame");
                continue;
            }
        };
        let frame = SseEvent::default().id(log_id.to_string()).data(data);
        if tx.send(Ok(frame)).await.is_err() {
            return false;
        }
    }
    true
}
