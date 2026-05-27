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
use crate::event_stream::{self, replay_matching_events, subscribe_ack_payload};
use crate::state::AppState;
use crate::subscribe_resume;

#[derive(Debug, Deserialize)]
pub struct McpStreamQuery {
    #[serde(default)]
    pub workspace_id: Option<uuid::Uuid>,
    /// Replay persisted events with `id > after_id` before live bus delivery.
    #[serde(default)]
    pub after_id: i64,
    #[serde(default)]
    pub resume_token: Option<String>,
}

pub async fn stream(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<McpStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    auth.require_capability(EVENT_SUBSCRIBE)
        .map_err(|_| ApiError::Forbidden("missing event:subscribe capability".into()))?;

    let (filter, after_id, from_resume_token) = resolve_stream_params(&state, &q, &auth)?;

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

    let mut high_water = after_id;
    if after_id > 0 || from_resume_token {
        high_water = replay_matching_events(state.store.as_ref(), &filter, after_id, &text_tx)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }

    let token = subscribe_resume::sign_resume_token(
        &filter,
        high_water,
        state.subscribe_resume_secret(),
        state.subscribe_resume_ttl_secs,
    )
    .map_err(|e| ApiError::Internal(format!("resume token: {e}")))?;
    let ack = subscribe_ack_payload(&token, high_water)
        .ok_or_else(|| ApiError::Internal("subscribe_ack serialization failed".into()))?;
    if text_tx.send(ack).await.is_err() {
        return Err(ApiError::Internal(
            "stream closed before subscribe_ack".into(),
        ));
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

fn resolve_stream_params(
    state: &AppState,
    q: &McpStreamQuery,
    auth: &AuthContext,
) -> Result<(EventFilter, i64, bool), ApiError> {
    if let Some(token) = q.resume_token.as_deref().filter(|t| !t.is_empty()) {
        if state.subscribe_resume_secret.is_none() && state.oidc.is_none() {
            return Err(ApiError::Internal(
                "subscribe resume not configured on server".into(),
            ));
        }
        let (filter, after_id) =
            subscribe_resume::verify_resume_token(token, state.subscribe_resume_secret())
                .map_err(|e| ApiError::BadRequest(format!("invalid resume_token: {e}")))?;
        if after_id > 0 && filter.workspace_id.is_none() {
            return Err(ApiError::BadRequest(
                "resume token requires workspace_id for replay".into(),
            ));
        }
        if let Some(ws) = filter.workspace_id {
            auth.ensure_workspace(ws)
                .map_err(|_| ApiError::Forbidden("token is not valid for this workspace".into()))?;
        }
        return Ok((filter, after_id, true));
    }

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

    Ok((filter, q.after_id, false))
}
