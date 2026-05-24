//! WebSocket `/ws/subscribe` handler.
//!
//! Protocol:
//! 1. Client opens `GET /ws/subscribe` and upgrades.
//! 2. Client sends one text frame with a JSON [`SubscribeFrame`] body.
//! 3. Server attaches a bus subscriber with the requested filter and
//!    streams matching events as text frames (one JSON object per frame).
//!    Each event frame includes `log_id` (persistent `maidan_events.id`)
//!    plus the usual externally-tagged event fields.
//! 4. When the subscriber lags the broadcast buffer, the server sends a
//!    `replay_hint` frame with `after_id` for `GET /workspaces/:wid/events`.
//! 5. Server pings every 30 s; if the client hasn't responded within
//!    `PONG_TIMEOUT` the server closes with code 1011.
//!
//! Close codes used:
//! - 1000 normal client-initiated close
//! - 1002 protocol error (bad first frame)
//! - 1008 policy violation (invalid filter JSON / auth failure)
//! - 1011 backpressure / pong timeout / unexpected stream end

use std::{borrow::Cow, time::Duration};

use axum::{
    extract::{
        ws::{CloseFrame, Message as WsMessage, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::StreamExt;
use maidan_auth::{capability::EVENT_SUBSCRIBE, resolve_bearer};
use maidan_bus::BusItem;
use maidan_types::EventFilter;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::mpsc,
    time::{timeout, Instant},
};

use crate::state::AppState;

const PING_INTERVAL: Duration = Duration::from_secs(30);
const PONG_TIMEOUT: Duration = Duration::from_secs(60);
const SEND_QUEUE: usize = 256;
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
pub struct SubscribeFrame {
    #[serde(default)]
    pub token: Option<String>,
    pub filter: EventFilter,
}

#[derive(Debug, Serialize)]
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

pub async fn subscribe(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run(socket, state))
}

async fn run(mut socket: WebSocket, state: AppState) {
    let filter = match read_subscribe(&mut socket, &state).await {
        Ok(f) => f,
        Err((code, reason)) => {
            let _ = socket
                .send(WsMessage::Close(Some(CloseFrame {
                    code,
                    reason: Cow::Owned(reason),
                })))
                .await;
            return;
        }
    };

    let mut subscriber = match state.bus.subscribe(filter.clone()).await {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(error = %err, "bus subscribe failed");
            let _ = socket
                .send(WsMessage::Close(Some(CloseFrame {
                    code: 1011,
                    reason: Cow::Borrowed("bus unavailable"),
                })))
                .await;
            return;
        }
    };

    let (tx, mut rx) = mpsc::channel::<WsMessage>(SEND_QUEUE);

    let bus_tx = tx.clone();
    let replay_workspace = filter.workspace_id;
    let bus_task = tokio::spawn(async move {
        let mut last_log_id: i64 = 0;
        while let Some(item) = subscriber.next().await {
            match item {
                BusItem::Event(envelope) => {
                    last_log_id = envelope.log_id;
                    let payload = match serde_json::to_string(envelope.as_ref()) {
                        Ok(p) => p,
                        Err(err) => {
                            tracing::warn!(error = %err, "drop unserializable event");
                            continue;
                        }
                    };
                    if bus_tx.send(WsMessage::Text(payload)).await.is_err() {
                        break;
                    }
                }
                BusItem::Lagged { skipped } => {
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
                    let payload = match serde_json::to_string(&hint) {
                        Ok(p) => p,
                        Err(err) => {
                            tracing::warn!(error = %err, "drop replay_hint");
                            continue;
                        }
                    };
                    if bus_tx.send(WsMessage::Text(payload)).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut last_pong = Instant::now();
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await;

    loop {
        tokio::select! {
            outbound = rx.recv() => {
                let Some(msg) = outbound else { break; };
                if socket.send(msg).await.is_err() {
                    break;
                }
            }
            inbound = socket.next() => {
                match inbound {
                    Some(Ok(WsMessage::Pong(_))) => last_pong = Instant::now(),
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        tracing::debug!(error = %err, "ws inbound error");
                        break;
                    }
                }
            }
            _ = ping_interval.tick() => {
                if last_pong.elapsed() > PONG_TIMEOUT {
                    let _ = socket
                        .send(WsMessage::Close(Some(CloseFrame {
                            code: 1011,
                            reason: Cow::Borrowed("pong timeout"),
                        })))
                        .await;
                    break;
                }
                if socket.send(WsMessage::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
        }
    }

    bus_task.abort();
}

async fn read_subscribe(
    socket: &mut WebSocket,
    state: &AppState,
) -> Result<EventFilter, (u16, String)> {
    let frame = timeout(FIRST_FRAME_TIMEOUT, socket.next())
        .await
        .map_err(|_| (1002u16, "subscribe frame timeout".to_string()))?;
    let frame = frame.ok_or_else(|| (1002u16, "connection closed before subscribe".to_string()))?;
    let frame = frame.map_err(|_| (1002u16, "ws read error".to_string()))?;

    let text = match frame {
        WsMessage::Text(t) => t,
        WsMessage::Close(_) => return Err((1000u16, String::new())),
        _ => return Err((1002u16, "expected text subscribe frame".to_string())),
    };

    let sub: SubscribeFrame =
        serde_json::from_str(&text).map_err(|e| (1008u16, format!("invalid subscribe: {e}")))?;

    if !state.auth_disabled {
        let secret = sub
            .token
            .as_deref()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| (1008u16, "missing token in subscribe frame".to_string()))?;
        let ctx = resolve_bearer(state.store.as_ref(), secret)
            .await
            .map_err(|_| (1008u16, "invalid or expired token".to_string()))?;
        ctx.require_capability(EVENT_SUBSCRIBE)
            .map_err(|_| (1008u16, "missing event:subscribe capability".to_string()))?;
    }

    Ok(sub.filter)
}
