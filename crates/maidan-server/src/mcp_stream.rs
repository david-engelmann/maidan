//! SSE event stream for MCP-style reactive clients (`GET /mcp/stream`).

use std::convert::Infallible;
use std::sync::{atomic::AtomicI64, Arc};
use std::time::Duration;

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    Extension,
};
use maidan_auth::{capability::EVENT_SUBSCRIBE, AuthContext};
use maidan_types::EventFilter;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::error::ApiError;
use crate::event_stream::{self, replay_matching_events};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct McpStreamQuery {
    #[serde(default)]
    pub workspace_id: Option<uuid::Uuid>,
    /// Replay persisted events with `id > after_id` before live bus delivery.
    #[serde(default)]
    pub after_id: i64,
}

pub async fn stream(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<McpStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    auth.require_capability(EVENT_SUBSCRIBE)
        .map_err(|_| ApiError::Forbidden("missing event:subscribe capability".into()))?;

    if q.after_id < 0 {
        return Err(ApiError::BadRequest("after_id must be non-negative".into()));
    }

    let mut filter = EventFilter::all();
    if let Some(ws) = q.workspace_id {
        filter.workspace_id = Some(maidan_types::WorkspaceId(ws));
        auth.ensure_workspace(maidan_types::WorkspaceId(ws))
            .map_err(|_| ApiError::Forbidden("token is not valid for this workspace".into()))?;
    } else if q.after_id > 0 {
        return Err(ApiError::BadRequest(
            "after_id requires workspace_id query parameter".into(),
        ));
    }

    let (sse_tx, sse_rx) = mpsc::channel(256);
    let (text_tx, mut text_rx) = mpsc::channel::<String>(256);

    tokio::spawn(async move {
        while let Some(payload) = text_rx.recv().await {
            if sse_tx
                .send(Ok(Event::default().data(payload)))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let mut high_water = q.after_id;
    if q.after_id > 0 {
        high_water = replay_matching_events(state.store.as_ref(), &filter, q.after_id, &text_tx)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }

    let bus_filter = filter.clone();
    let subscriber = state
        .bus
        .subscribe(filter)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let watermark = Arc::new(AtomicI64::new(high_water));
    let bus_store = state.store.clone();
    tokio::spawn(async move {
        event_stream::forward_bus_items(subscriber, text_tx, watermark, bus_store, bus_filter)
            .await;
    });

    let stream = ReceiverStream::new(sse_rx);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}
