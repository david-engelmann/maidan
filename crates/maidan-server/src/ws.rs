//! WebSocket `/ws/subscribe` handler.
//!
//! Protocol:
//! 1. Client opens `GET /ws/subscribe` and upgrades.
//! 2. Client sends one text frame with a JSON [`SubscribeFrame`] body
//!    (optional `consumer_id` for delivery cursor replay floor).
//! 3. Optional `after_id` or `resume_token` replays matching rows from
//!    `maidan_events` (requires `filter.workspace_id`), then a
//!    `subscribe_ack` with a signed `resume_token`, then live bus events.
//! 4. Each event frame includes `log_id` plus the externally-tagged event.
//! 5. On broadcast lag, replay from `maidan_events` when `filter.workspace_id`
//!    is set; otherwise a `replay_hint` frame (see 1.1.2 / 3.0.2).
//! 6. Server pings every 30 s; pong timeout closes with 1011.
//! 7. Optional `member_id` with `filter.workspace_id` enables ephemeral
//!    `presence` / `typing` frames (see [`crate::presence`]).
//! 8. After subscribe, client may send `{"type":"presence","status":"online"|"away"}`
//!    or `{"type":"typing","thread_id":"…","active":true|false}`.
//! 9. The credential that opened the stream is checked again while it runs:
//!    before a frame goes out or a client frame is acted on (at most once per
//!    [`RECHECK_GAP`]), and at every ping. A revoked or expired token, an ended
//!    session, or a member deactivated through SCIM closes the stream with 1008.

use std::{sync::Arc, time::Duration};

use axum::{
    extract::{
        ws::{CloseFrame, Message as WsMessage, WebSocket},
        State, WebSocketUpgrade,
    },
    http::HeaderMap,
    response::IntoResponse,
};
use futures::StreamExt;
use maidan_auth::{
    capability::EVENT_SUBSCRIBE,
    resolve_bearer,
    subscribe::{resolve_subscribe, SubscribeFrame, SubscribeRefusal},
    AuthContext,
};
use maidan_store::StoreError;
use maidan_types::{ApiTokenId, EventFilter, LogSnapshot, MemberId, ThreadId, WorkspaceId};
use serde::Deserialize;
use tokio::{
    sync::mpsc,
    time::{timeout, Instant},
};
use uuid::Uuid;

use crate::event_stream::{
    self, emit_replay_truncated_if_needed, replay_matching_events, subscribe_ack_payload,
};
use crate::session::load_session;
use crate::state::AppState;

const PING_INTERVAL: Duration = Duration::from_secs(30);
const PONG_TIMEOUT: Duration = Duration::from_secs(60);
const SEND_QUEUE: usize = 256;
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// The shortest time between two checks of a running stream's credential. A
/// burst of frames costs one check, and a deactivated member's stream still
/// ends at its next frame once this much time has passed.
const RECHECK_GAP: Duration = Duration::from_secs(1);

/// What opened the stream, kept so the stream can be checked again while it
/// runs. Deactivating a member revokes their tokens and ends their sessions
/// on the next request, but a stream makes no further requests, so without
/// this it outlived both.
#[derive(Clone)]
enum StreamCredential {
    /// Auth is disabled: nothing to check.
    Open,
    /// A bearer token in the subscribe frame.
    Token {
        token_id: ApiTokenId,
        member_id: MemberId,
    },
    /// The browser session cookie from the upgrade request.
    Session {
        cookie: HeaderMap,
        member_id: MemberId,
    },
}

impl StreamCredential {
    /// `Err(reason)` when the stream must end. A failed lookup ends it too,
    /// since it cannot show the credential still holds; the client reconnects
    /// and is checked afresh.
    async fn recheck(&self, state: &AppState) -> Result<(), &'static str> {
        let member_id = match self {
            Self::Open => return Ok(()),
            Self::Token {
                token_id,
                member_id,
            } => {
                maidan_auth::resolve_token_id(state.store.as_ref(), *token_id)
                    .await
                    .map_err(|_| "token no longer valid")?;
                *member_id
            }
            Self::Session { cookie, member_id } => {
                let session = load_session(state, cookie)
                    .await
                    .map_err(|_| "session ended")?;
                if session.member_id != *member_id {
                    return Err("session ended");
                }
                *member_id
            }
        };
        match state.store.get_scim_user(member_id).await {
            Ok(Some(user)) if !user.active => Err("member deactivated"),
            Ok(_) => Ok(()),
            Err(_) => Err("credential check failed"),
        }
    }
}

struct SubscribeRequest {
    filter: EventFilter,
    after_id: i64,
    from_resume_token: bool,
    consumer_id: Option<String>,
    member_id: Option<MemberId>,
    at_least_once: bool,
    lean: bool,
    /// Who the socket acts as, from its credential; `None` when auth is off.
    attribution: Option<maidan_types::Attribution>,
    /// The credential, checked again while the stream runs.
    credential: StreamCredential,
}

/// Fail-loud subscribe rejection. A too-old cursor sends a JSON frame
/// (`type: cursor_too_old`, `must_refetch: true`) then closes 1008 — never a
/// silent clamp. Other rejects only close.
struct SubscribeReject {
    code: u16,
    reason: String,
    frame: Option<String>,
}

impl From<(u16, String)> for SubscribeReject {
    fn from((code, reason): (u16, String)) -> Self {
        Self {
            code,
            reason,
            frame: None,
        }
    }
}

impl SubscribeReject {
    fn cursor_too_old(after_id: i64, oldest_id: i64, workspace_id: Option<WorkspaceId>) -> Self {
        let mut frame = serde_json::json!({
            "type": "cursor_too_old",
            "after_id": after_id,
            "oldest_id": oldest_id,
            "must_refetch": true,
        });
        if let Some(ws) = workspace_id {
            frame["snapshot"] = serde_json::json!(LogSnapshot::path(ws));
        }
        Self {
            code: 1008,
            reason: "cursor_too_old".into(),
            frame: Some(frame.to_string()),
        }
    }

    fn from_store(err: StoreError, workspace_id: Option<WorkspaceId>) -> Self {
        match err {
            StoreError::CursorTooOld {
                after_id,
                oldest_id,
            } => Self::cursor_too_old(after_id, oldest_id, workspace_id),
            other => (1011u16, other.to_string()).into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientWsFrame {
    Presence { status: String },
    Typing { thread_id: Uuid, active: bool },
}

/// The largest frame or message a subscriber may send. Clients send only a
/// subscribe frame and small presence/typing frames, so this is generous; the
/// transport default (64 MiB) would let one frame bypass the request-body
/// limit every REST route is held to.
pub const MAX_CLIENT_WS_MESSAGE_BYTES: usize = 64 * 1024;

/// Holds one slot in the connection count for as long as the socket lives.
struct ConnectionSlot(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

pub async fn subscribe(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    // Long-lived connections were uncapped: each holds a task, buffers and a
    // bus subscription, so enough of them exhaust the process. Past the
    // ceiling a client is told to come back rather than degrading everyone.
    let count = state.ws_connections.clone();
    if count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= state.max_ws_connections {
        count.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        let mut response = crate::error::ApiError::Overloaded(
            "the server is at its subscriber connection limit; retry after the Retry-After delay"
                .to_string(),
        )
        .into_response();
        response.headers_mut().insert(
            axum::http::header::RETRY_AFTER,
            axum::http::HeaderValue::from_static("5"),
        );
        return response;
    }
    let slot = ConnectionSlot(count);
    // The upgrade runs on a task hyper spawns, which does not inherit this
    // request's trace. Capture it and re-enter on the socket task.
    let trace = maidan_store::trace::current();
    ws.max_message_size(MAX_CLIENT_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_CLIENT_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            maidan_store::trace::maybe_scope(trace, run(socket, state, headers)).await
        })
        .into_response()
}

async fn run(mut socket: WebSocket, state: AppState, headers: HeaderMap) {
    let request = match read_subscribe(&mut socket, &state, &headers).await {
        Ok(r) => r,
        Err(reject) => {
            if let Some(frame) = reject.frame {
                let _ = socket.send(WsMessage::Text(frame.into())).await;
            }
            let _ = socket
                .send(WsMessage::Close(Some(CloseFrame {
                    code: reject.code,
                    reason: reject.reason.into(),
                })))
                .await;
            return;
        }
    };
    // The socket outlives the upgrade request and is authenticated by its first
    // frame, so no request layer scoped it: what it writes (the last-seen
    // stamp, delivery cursors) is the subscriber's, scoped here.
    let attribution = request.attribution;
    maidan_store::attribution::with_attribution(attribution, serve(socket, state, request)).await
}

async fn serve(mut socket: WebSocket, state: AppState, request: SubscribeRequest) {
    let (tx, mut rx) = mpsc::channel::<WsMessage>(SEND_QUEUE);

    let text_tx = {
        let ws_tx = tx.clone();
        let (text_tx, mut text_rx) = mpsc::channel::<String>(SEND_QUEUE);
        maidan_store::attribution::spawn(async move {
            while let Some(payload) = text_rx.recv().await {
                if ws_tx.send(WsMessage::Text(payload.into())).await.is_err() {
                    break;
                }
            }
        });
        text_tx
    };

    // Subscribe before the replay and the ack. An event published after the
    // replay's read then waits in the subscription, and the watermark drops
    // the ones the replay already sent. Subscribing after the replay lost
    // every event published in between. It also makes the ack a promise that
    // the client is live.
    let subscriber = match state.bus.subscribe(request.filter.clone()).await {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(error = %err, "bus subscribe failed");
            let _ = socket
                .send(WsMessage::Close(Some(CloseFrame {
                    code: 1011,
                    reason: "bus unavailable".into(),
                })))
                .await;
            return;
        }
    };

    let mut high_water = request.after_id;
    // At-least-once mode delivers the backlog via the reconcile loop's first
    // pass (stability-gated), so skip the optimistic replay here.
    if !request.at_least_once && (request.after_id > 0 || request.from_resume_token) {
        match replay_matching_events(
            state.store.as_ref(),
            &request.filter,
            request.after_id,
            &text_tx,
            request.consumer_id.as_deref(),
            request.lean,
        )
        .await
        {
            Ok(outcome) => {
                high_water = outcome.high_water;
                emit_replay_truncated_if_needed(
                    &text_tx,
                    outcome.high_water,
                    request.filter.workspace_id,
                    outcome.truncated,
                )
                .await;
            }
            Err(err) => {
                tracing::warn!(error = %err, "ws replay from event log failed");
                let _ = socket
                    .send(WsMessage::Close(Some(CloseFrame {
                        code: 1011,
                        reason: "replay failed".into(),
                    })))
                    .await;
                return;
            }
        }
    }

    match send_subscribe_ack(&state, &request.filter, high_water, &text_tx).await {
        Ok(()) => {}
        Err(reason) => {
            let _ = socket
                .send(WsMessage::Close(Some(CloseFrame {
                    code: 1011,
                    reason: reason.into(),
                })))
                .await;
            return;
        }
    }

    let bus_filter = request.filter.clone();
    let bus_store = state.store.clone();
    let bus_tx = text_tx.clone();
    let delivery_consumer_id = request.consumer_id.clone();
    let delivery_stability = state.delivery_stability;
    let delivery_reconcile_interval = state.delivery_reconcile_interval;
    let lean = request.lean;
    // `at_least_once` is only set when both workspace_id and consumer_id resolve
    // (see `resolve_subscribe_request`); the destructure falls back otherwise.
    let reconcile = request
        .at_least_once
        .then(|| {
            request
                .filter
                .workspace_id
                .zip(delivery_consumer_id.clone())
        })
        .flatten();
    let bus_task = if let Some((workspace_id, consumer_id)) = reconcile {
        maidan_store::attribution::spawn(async move {
            event_stream::reconcile_deliver(
                subscriber,
                bus_tx,
                bus_store,
                bus_filter,
                workspace_id,
                consumer_id,
                high_water,
                delivery_stability,
                delivery_reconcile_interval,
                lean,
            )
            .await;
        })
    } else {
        let watermark = Arc::new(std::sync::atomic::AtomicI64::new(high_water));
        maidan_store::attribution::spawn(async move {
            event_stream::forward_bus_items(
                subscriber,
                bus_tx,
                watermark,
                bus_store,
                bus_filter,
                crate::subscribe_metrics::SubscribeTransport::Ws,
                delivery_consumer_id,
                lean,
            )
            .await;
        })
    };

    let _presence_reg = match (request.filter.workspace_id, request.member_id) {
        (Some(workspace_id), Some(member_id)) => {
            // Durable last-seen: record that this member is connected right
            // now, so presence-aware email routing can tell whether they are
            // active — a signal that survives a restart and is visible across
            // replicas, unlike the in-memory `PresenceHub`. Best-effort +
            // spawned so a store hiccup never blocks the connect.
            {
                let store = state.store.clone();
                maidan_store::attribution::spawn(async move {
                    if let Err(err) = store.touch_member_last_seen(member_id).await {
                        tracing::warn!(error = %err, "presence last-seen touch failed");
                    }
                });
            }
            let (mut ephemeral_rx, reg, snapshot) =
                state.presence.register(workspace_id, member_id);
            let _ = text_tx.send(snapshot).await;
            let ephemeral_tx = text_tx.clone();
            let hub = state.presence.clone();
            maidan_store::attribution::spawn(async move {
                loop {
                    let frame = match ephemeral_rx.recv().await {
                        Ok(msg) => msg,
                        // Diffs were dropped, so the client's roster is wrong
                        // in a way no later diff repairs. A fresh snapshot
                        // supersedes everything missed.
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            hub.snapshot(workspace_id)
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    };
                    if ephemeral_tx.send(frame).await.is_err() {
                        break;
                    }
                }
            });
            Some(reg)
        }
        _ => None,
    };

    let presence_hub = state.presence.clone();
    let presence_workspace = request.filter.workspace_id;
    let presence_member = request.member_id;

    let mut last_pong = Instant::now();
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await;
    let credential = request.credential;
    let mut last_check = Instant::now();

    loop {
        tokio::select! {
            outbound = rx.recv() => {
                let Some(msg) = outbound else { break; };
                if last_check.elapsed() >= RECHECK_GAP {
                    last_check = Instant::now();
                    if let Err(reason) = credential.recheck(&state).await {
                        close_ended(&mut socket, reason).await;
                        break;
                    }
                }
                if socket.send(msg).await.is_err() {
                    break;
                }
            }
            inbound = socket.next() => {
                match inbound {
                    Some(Ok(WsMessage::Pong(_))) => last_pong = Instant::now(),
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(WsMessage::Text(text))) => {
                        // Presence and typing speak for the member: not after
                        // the credential has ended.
                        if last_check.elapsed() >= RECHECK_GAP {
                            last_check = Instant::now();
                            if let Err(reason) = credential.recheck(&state).await {
                                close_ended(&mut socket, reason).await;
                                break;
                            }
                        }
                        if let Ok(frame) = serde_json::from_str::<ClientWsFrame>(&text) {
                            handle_client_frame(
                                &presence_hub,
                                presence_workspace,
                                presence_member,
                                &frame,
                            );
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        tracing::debug!(error = %err, "ws inbound error");
                        break;
                    }
                }
            }
            _ = ping_interval.tick() => {
                last_check = Instant::now();
                if let Err(reason) = credential.recheck(&state).await {
                    close_ended(&mut socket, reason).await;
                    break;
                }
                if last_pong.elapsed() > PONG_TIMEOUT {
                    let _ = socket
                        .send(WsMessage::Close(Some(CloseFrame {
                            code: 1011,
                            reason: "pong timeout".into(),
                        })))
                        .await;
                    break;
                }
                if socket.send(WsMessage::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
        }
    }

    drop(text_tx);
    bus_task.abort();
}

/// Close a stream whose credential no longer holds (1008, policy violation).
async fn close_ended(socket: &mut WebSocket, reason: &'static str) {
    let _ = socket
        .send(WsMessage::Close(Some(CloseFrame {
            code: 1008,
            reason: reason.into(),
        })))
        .await;
}

async fn send_subscribe_ack(
    state: &AppState,
    filter: &EventFilter,
    after_id: i64,
    text_tx: &mpsc::Sender<String>,
) -> Result<(), String> {
    let secret = state
        .subscribe_resume_secret()
        .ok_or_else(|| "subscribe resume not configured on server".to_string())?;
    let token = maidan_auth::subscribe::sign_resume_token(
        filter,
        after_id,
        secret,
        state.subscribe_resume_ttl_secs,
    )
    .map_err(|e| format!("resume token: {e}"))?;
    let payload = subscribe_ack_payload(
        &token,
        after_id,
        crate::room_lsn::current_for_scope(state.store.as_ref(), filter.workspace_id).await,
    )
    .ok_or_else(|| "subscribe_ack serialization failed".to_string())?;
    text_tx
        .send(payload)
        .await
        .map_err(|_| "client disconnected before subscribe_ack".to_string())
}

async fn read_subscribe(
    socket: &mut WebSocket,
    state: &AppState,
    headers: &HeaderMap,
) -> Result<SubscribeRequest, SubscribeReject> {
    let frame = timeout(FIRST_FRAME_TIMEOUT, socket.next())
        .await
        .map_err(|_| (1002u16, "subscribe frame timeout".to_string()))?;
    let frame = frame.ok_or_else(|| (1002u16, "connection closed before subscribe".to_string()))?;
    let frame = frame.map_err(|_| (1002u16, "ws read error".to_string()))?;

    let text = match frame {
        WsMessage::Text(t) => t,
        WsMessage::Close(_) => return Err((1000u16, String::new()).into()),
        _ => return Err((1002u16, "expected text subscribe frame".to_string()).into()),
    };

    let sub: SubscribeFrame =
        serde_json::from_str(&text).map_err(|e| (1008u16, format!("invalid subscribe: {e}")))?;

    let (mut filter, mut after_id) = resolve_subscribe(&sub, state.subscribe_resume_secret())
        .map_err(|refusal| {
            let code = match refusal {
                SubscribeRefusal::ResumeNotConfigured => 1011u16,
                _ => 1008,
            };
            (code, refusal.to_string())
        })?;
    // Resolve the caller's identity *before* expanding the filter / applying
    // channel grants, so the DM-participant check and the grant verification
    // can both check real membership.
    let (ctx, credential) = if state.auth_disabled {
        (AuthContext::bypass(), StreamCredential::Open)
    } else if let Some(secret) = sub.token.as_deref().filter(|t| !t.is_empty()) {
        let ctx = resolve_bearer(state.store.as_ref(), secret)
            .await
            .map_err(|_| (1008u16, "invalid or expired token".to_string()))?;
        let credential = match ctx.token_id {
            Some(token_id) => StreamCredential::Token {
                token_id,
                member_id: ctx.member_id,
            },
            None => return Err((1008u16, "invalid or expired token".to_string()).into()),
        };
        (ctx, credential)
    } else if let Ok(session) = load_session(state, headers).await {
        // The handshake is a GET, but another origin could read this stream.
        crate::session::refuse_cross_origin(headers).map_err(|_| {
            (
                1008u16,
                "a cross-origin socket cannot use a browser session".to_string(),
            )
        })?;
        let mut cookie = HeaderMap::new();
        if let Some(value) = headers.get(axum::http::header::COOKIE) {
            cookie.insert(axum::http::header::COOKIE, value.clone());
        }
        let credential = StreamCredential::Session {
            cookie,
            member_id: session.member_id,
        };
        (
            session.auth_context(crate::auth::OIDC_READ_CAPABILITIES),
            credential,
        )
    } else {
        return Err((
            1008u16,
            "missing token in subscribe frame or browser session".into(),
        )
            .into());
    };
    crate::dm::expand_event_filter(state, &mut filter, &ctx)
        .await
        .map_err(|e| (1008u16, format!("{e:?}")))?;
    if !state.auth_disabled {
        ctx.require_capability(EVENT_SUBSCRIBE)
            .map_err(|_| (1008u16, "missing event:subscribe capability".to_string()))?;
    }
    crate::subscribe_grants::bind_to_caller_workspace(&mut filter, &ctx)
        .map_err(|e| (1008u16, e.to_string()))?;
    crate::subscribe_grants::apply_subscribe_grants(state, &ctx, &mut filter)
        .await
        .map_err(|e| (1008u16, e))?;
    if let Some(ref consumer_id) = sub.consumer_id {
        crate::delivery::validate_consumer_id(consumer_id).map_err(|e| (1008u16, e))?;
        after_id = crate::delivery::effective_subscribe_after_id(
            state.store.as_ref(),
            Some(consumer_id.as_str()),
            filter.workspace_id,
            after_id,
        )
        .await
        .map_err(|e| SubscribeReject::from_store(e, filter.workspace_id))?;
    }
    crate::delivery::ensure_subscribe_cursor(state.store.as_ref(), filter.workspace_id, after_id)
        .await
        .map_err(|e| SubscribeReject::from_store(e, filter.workspace_id))?;

    // Presence and typing speak for a member, so the frame may name only the
    // member the credential is. Taken from the frame unchecked, a token for one
    // member could show another as online or typing, and stamp any member's
    // last-seen time, in any workspace, which quiets presence-aware email.
    let member_id = match sub.member_id.map(MemberId) {
        Some(claimed) if !state.auth_disabled && claimed != ctx.member_id => {
            return Err((
                1008u16,
                "member_id must be the member the token or session belongs to".into(),
            )
                .into());
        }
        claimed => claimed,
    };
    if member_id.is_some() && filter.workspace_id.is_none() {
        return Err((
            1008u16,
            "member_id requires filter.workspace_id for presence".into(),
        )
            .into());
    }

    // At-least-once requires both a workspace filter and a durable consumer id
    // (the reconcile cursor is keyed by them); silently ignore the flag otherwise.
    let at_least_once =
        sub.at_least_once && filter.workspace_id.is_some() && sub.consumer_id.is_some();

    Ok(SubscribeRequest {
        filter,
        after_id,
        from_resume_token: sub.resume_token.as_deref().is_some_and(|t| !t.is_empty()),
        consumer_id: sub.consumer_id.clone(),
        member_id,
        at_least_once,
        lean: sub.lean,
        attribution: ctx.attribution(),
        credential,
    })
}

fn handle_client_frame(
    hub: &crate::presence::PresenceHub,
    workspace_id: Option<WorkspaceId>,
    member_id: Option<MemberId>,
    frame: &ClientWsFrame,
) {
    let (Some(workspace_id), Some(member_id)) = (workspace_id, member_id) else {
        return;
    };
    match frame {
        ClientWsFrame::Presence { status } => {
            if let Some(st) = crate::presence::PresenceStatus::parse(status) {
                hub.set_presence(workspace_id, member_id, st);
            }
        }
        ClientWsFrame::Typing { thread_id, active } => {
            hub.set_typing(workspace_id, ThreadId(*thread_id), member_id, *active);
        }
    }
}
