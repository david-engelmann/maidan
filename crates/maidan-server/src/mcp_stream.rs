//! SSE event stream for MCP-style reactive clients (`GET /mcp/stream`).

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    Extension,
};
use futures::StreamExt;
use maidan_auth::{capability::EVENT_SUBSCRIBE, AuthContext};
use maidan_bus::BusItem;
use maidan_types::EventFilter;
use serde::Deserialize;
use serde::Serialize;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct McpStreamQuery {
    #[serde(default)]
    pub workspace_id: Option<uuid::Uuid>,
}

pub async fn stream(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<McpStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    auth.require_capability(EVENT_SUBSCRIBE)
        .map_err(|_| ApiError::Forbidden("missing event:subscribe capability".into()))?;

    let mut filter = EventFilter::all();
    if let Some(ws) = q.workspace_id {
        filter.workspace_id = Some(maidan_types::WorkspaceId(ws));
        auth.ensure_workspace(maidan_types::WorkspaceId(ws))
            .map_err(|_| ApiError::Forbidden("token is not valid for this workspace".into()))?;
    }

    let replay_workspace = filter.workspace_id;
    let mut subscriber = state
        .bus
        .subscribe(filter)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let (tx, rx) = tokio::sync::mpsc::channel(256);

    tokio::spawn(async move {
        let mut last_log_id: i64 = 0;
        while let Some(item) = subscriber.next().await {
            let data = match item {
                BusItem::Event(envelope) => {
                    last_log_id = envelope.log_id;
                    match serde_json::to_string(envelope.as_ref()) {
                        Ok(s) => s,
                        Err(err) => {
                            tracing::warn!(error = %err, "drop unserializable event for sse");
                            continue;
                        }
                    }
                }
                BusItem::Lagged { skipped } => {
                    #[derive(Serialize)]
                    struct ReplayHint {
                        #[serde(rename = "type")]
                        frame_type: &'static str,
                        skipped: u64,
                        after_id: i64,
                        #[serde(skip_serializing_if = "Option::is_none")]
                        workspace_id: Option<uuid::Uuid>,
                        #[serde(skip_serializing_if = "Option::is_none")]
                        replay: Option<String>,
                    }
                    let hint = ReplayHint {
                        frame_type: "replay_hint",
                        skipped,
                        after_id: last_log_id,
                        workspace_id: replay_workspace.map(|w| w.0),
                        replay: replay_workspace.map(|w| {
                            format!(
                                "/workspaces/{}/events?after_id={last_log_id}&limit=100",
                                w.0
                            )
                        }),
                    };
                    match serde_json::to_string(&hint) {
                        Ok(s) => s,
                        Err(err) => {
                            tracing::warn!(error = %err, "drop replay_hint for sse");
                            continue;
                        }
                    }
                }
            };
            if tx.send(Ok(Event::default().data(data))).await.is_err() {
                break;
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}
