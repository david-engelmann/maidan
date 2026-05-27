//! WebSocket `/ws/subscribe` handler.
//!
//! Protocol:
//! 1. Client opens `GET /ws/subscribe` and upgrades.
//! 2. Client sends one text frame with a JSON [`SubscribeFrame`] body.
//! 3. Optional `after_id` replays matching rows from `maidan_events` (requires
//!    `filter.workspace_id`), then live bus events with `log_id` greater than
//!    the replay watermark.
//! 4. Each event frame includes `log_id` plus the externally-tagged event.
//! 5. On broadcast lag, replay from `maidan_events` when `filter.workspace_id`
//!    is set; otherwise a `replay_hint` frame (see 1.1.2 / 3.0.2).
//! 6. Server pings every 30 s; pong timeout closes with 1011.

use std::{borrow::Cow, sync::Arc, time::Duration};

use axum::{
    extract::{
        ws::{CloseFrame, Message as WsMessage, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::StreamExt;
use maidan_auth::{capability::EVENT_SUBSCRIBE, resolve_bearer};
use maidan_types::EventFilter;
use serde::Deserialize;
use tokio::{
    sync::mpsc,
    time::{timeout, Instant},
};

use crate::event_stream::{self, replay_matching_events};
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
    /// Replay persisted events with `id > after_id` before attaching to the bus.
    #[serde(default)]
    pub after_id: i64,
}

struct SubscribeRequest {
    filter: EventFilter,
    after_id: i64,
}

pub async fn subscribe(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run(socket, state))
}

async fn run(mut socket: WebSocket, state: AppState) {
    let request = match read_subscribe(&mut socket, &state).await {
        Ok(r) => r,
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

    let (tx, mut rx) = mpsc::channel::<WsMessage>(SEND_QUEUE);

    let text_tx = {
        let ws_tx = tx.clone();
        let (text_tx, mut text_rx) = mpsc::channel::<String>(SEND_QUEUE);
        tokio::spawn(async move {
            while let Some(payload) = text_rx.recv().await {
                if ws_tx.send(WsMessage::Text(payload)).await.is_err() {
                    break;
                }
            }
        });
        text_tx
    };

    let mut high_water = request.after_id;
    if request.after_id > 0 {
        match replay_matching_events(
            state.store.as_ref(),
            &request.filter,
            request.after_id,
            &text_tx,
        )
        .await
        {
            Ok(hw) => high_water = hw,
            Err(err) => {
                tracing::warn!(error = %err, "ws replay from event log failed");
                let _ = socket
                    .send(WsMessage::Close(Some(CloseFrame {
                        code: 1011,
                        reason: Cow::Borrowed("replay failed"),
                    })))
                    .await;
                return;
            }
        }
    }

    let subscriber = match state.bus.subscribe(request.filter.clone()).await {
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

    let watermark = Arc::new(std::sync::atomic::AtomicI64::new(high_water));
    let bus_filter = request.filter.clone();
    let bus_store = state.store.clone();
    let bus_tx = text_tx.clone();
    let bus_task = tokio::spawn(async move {
        event_stream::forward_bus_items(subscriber, bus_tx, watermark, bus_store, bus_filter).await;
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

    drop(text_tx);
    bus_task.abort();
}

async fn read_subscribe(
    socket: &mut WebSocket,
    state: &AppState,
) -> Result<SubscribeRequest, (u16, String)> {
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

    if sub.after_id < 0 {
        return Err((1008u16, "after_id must be non-negative".into()));
    }
    if sub.after_id > 0 && sub.filter.workspace_id.is_none() {
        return Err((
            1008u16,
            "after_id requires filter.workspace_id for replay".into(),
        ));
    }

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
        if let Some(ws) = sub.filter.workspace_id {
            ctx.ensure_workspace(ws)
                .map_err(|_| (1008u16, "token is not valid for this workspace".into()))?;
        }
    }

    Ok(SubscribeRequest {
        filter: sub.filter,
        after_id: sub.after_id,
    })
}
