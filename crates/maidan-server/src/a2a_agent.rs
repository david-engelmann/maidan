//! Google A2A protocol v1.0 JSON-RPC ingress (`POST /a2a/v1/rpc`).

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::Event as SseEvent;
use axum::response::{IntoResponse, Response, Sse};
use axum::{extract::State, Extension, Json};
use chrono::Utc;
use futures::StreamExt;
use maidan_a2a::{
    is_terminal_task_state, maidan_context_from_metadata, message_text,
    GetPushNotificationConfigResponse, GetTaskRequest, JsonRpcId, JsonRpcRequest, JsonRpcResponse,
    SendMessageRequest, SendMessageResponse, SetPushNotificationConfigRequest,
    StreamResponseStatusUpdate, StreamResponseTask, Task, TaskStatus, TaskStatusUpdateEvent,
    METHOD_GET_PUSH_NOTIFICATION_CONFIG, METHOD_GET_TASK, METHOD_SEND_MESSAGE,
    METHOD_SEND_STREAMING_MESSAGE, METHOD_SET_PUSH_NOTIFICATION_CONFIG, METHOD_SUBSCRIBE_TO_TASK,
    METHOD_TASKS_CANCEL, METHOD_TASKS_RESUBSCRIBE, TASK_STATE_CANCELED, TASK_STATE_COMPLETED,
    TASK_STATE_WORKING,
};
use maidan_auth::capability::MESSAGE_POST;
use maidan_auth::capability::WORKSPACE_WRITE;
use maidan_auth::AuthContext;
use maidan_router::resolve_thread_context;
use maidan_types::{Event, *};
use serde::Serialize;
use uuid::Uuid;

use crate::routes::publish;
use crate::state::AppState;

const ERR_PARSE: i32 = -32700;
const ERR_METHOD: i32 = -32601;
const ERR_PARAMS: i32 = -32602;
const ERR_INTERNAL: i32 = -32603;
const ERR_UNSUPPORTED: i32 = -32005;
const SUBSCRIBE_POLL_MS: u64 = 100;
const SUBSCRIBE_MAX_POLLS: u32 = 300;

pub async fn json_rpc(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<JsonRpcRequest>,
) -> Response {
    if body.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            body.id,
            ERR_PARSE,
            "jsonrpc must be \"2.0\"",
        ))
        .into_response();
    }
    let id = body.id.clone();
    match body.method.as_str() {
        METHOD_SEND_MESSAGE => {
            json_response(dispatch_send_message(&state, &auth, id, body.params).await)
        }
        METHOD_SEND_STREAMING_MESSAGE => {
            dispatch_send_streaming_message(&state, &auth, id, body.params).await
        }
        METHOD_GET_TASK => json_response(dispatch_get_task(&state, &auth, id, body.params).await),
        METHOD_TASKS_CANCEL => {
            json_response(dispatch_tasks_cancel(&state, &auth, id, body.params).await)
        }
        METHOD_SET_PUSH_NOTIFICATION_CONFIG => {
            json_response(dispatch_set_push_config(&state, &auth, id, body.params).await)
        }
        METHOD_GET_PUSH_NOTIFICATION_CONFIG => {
            json_response(dispatch_get_push_config(&state, &auth, id).await)
        }
        METHOD_SUBSCRIBE_TO_TASK | METHOD_TASKS_RESUBSCRIBE => {
            dispatch_subscribe_to_task(&state, &auth, id, body.params).await
        }
        other => json_response(Ok(JsonRpcResponse::error(
            id,
            ERR_METHOD,
            format!("method not found: {other}"),
        ))),
    }
}

fn json_response(result: Result<JsonRpcResponse, JsonRpcResponse>) -> Response {
    match result {
        Ok(resp) => Json(resp).into_response(),
        Err(resp) => Json(resp).into_response(),
    }
}

async fn persist_task(state: &AppState, workspace_id: WorkspaceId, task: &Task) {
    let Ok(value) = serde_json::to_value(task) else {
        return;
    };
    if state
        .store
        .upsert_a2a_task(workspace_id, &task.id, value.clone())
        .await
        .is_err()
    {
        return;
    }
    if let Ok(Some(url)) = state.store.get_a2a_push_config(workspace_id).await {
        let task_id = task.id.clone();
        tokio::spawn(async move {
            deliver_a2a_push(&url, &value, &task_id).await;
        });
    }
}

/// Deliver an A2A push notification with bounded retry + backoff. Best-effort
/// (not a durable outbox), but — unlike the prior fire-and-forget — failures are
/// retried, logged, and counted (`maidan_a2a_push_total{result}`) so a dropped
/// agent notification is visible instead of silent.
async fn deliver_a2a_push(url: &str, value: &serde_json::Value, task_id: &str) {
    const MAX_ATTEMPTS: u32 = 3;
    let client = reqwest::Client::new();
    let mut backoff = Duration::from_millis(200);
    for attempt in 1..=MAX_ATTEMPTS {
        match client
            .post(url)
            .header("Content-Type", "application/json")
            .json(value)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                metrics::counter!("maidan_a2a_push_total", "result" => "ok").increment(1);
                return;
            }
            Ok(resp) => {
                tracing::warn!(task_id, attempt, status = %resp.status(), "a2a push got non-success status");
            }
            Err(err) => {
                tracing::warn!(task_id, attempt, error = %err, "a2a push request failed");
            }
        }
        if attempt < MAX_ATTEMPTS {
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(2));
        }
    }
    metrics::counter!("maidan_a2a_push_total", "result" => "failed").increment(1);
    tracing::error!(
        task_id,
        attempts = MAX_ATTEMPTS,
        "a2a push gave up after retries"
    );
}

async fn load_task(state: &AppState, task_id: &str) -> Result<Task, String> {
    let value = state
        .store
        .get_a2a_task(task_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn sse_json_rpc_frame(frame: JsonRpcResponse) -> SseEvent {
    let data = serde_json::to_string(&frame).unwrap_or_else(|err| {
        tracing::error!(error = %err, "a2a: failed to serialize JSON-RPC SSE frame");
        String::new()
    });
    SseEvent::default().data(data)
}

async fn ensure_task_workspace_access(
    state: &AppState,
    auth: &AuthContext,
    id: &JsonRpcId,
    task_id: &str,
    task: &Task,
) -> Result<Option<WorkspaceId>, JsonRpcResponse> {
    if let Ok(Some(ws)) = state.store.get_a2a_task_workspace(task_id).await {
        if let Err(e) = auth.ensure_workspace(ws) {
            return Err(JsonRpcResponse::error(id.clone(), -32001, e.to_string()));
        }
        return Ok(Some(ws));
    }
    if let Some(context_id) = &task.context_id {
        if let Ok(thread_id) = uuid::Uuid::parse_str(context_id) {
            if let Ok(thread_ctx) =
                resolve_thread_context(state.store.as_ref(), ThreadId(thread_id)).await
            {
                if let Err(e) = auth.ensure_workspace(thread_ctx.workspace_id) {
                    return Err(JsonRpcResponse::error(id.clone(), -32001, e.to_string()));
                }
                // Cluster 179: per-channel access on the A2A read path too — a
                // task's context thread may live in a private channel.
                if let Err(e) = maidan_auth::ensure_channel_access(
                    state.store.as_ref(),
                    auth,
                    thread_ctx.channel_id,
                )
                .await
                {
                    return Err(JsonRpcResponse::error(id.clone(), -32001, e.to_string()));
                }
                return Ok(Some(thread_ctx.workspace_id));
            }
        }
    }
    Ok(None)
}

struct PostedA2a {
    task_id: String,
    workspace_id: WorkspaceId,
    thread_id: Uuid,
    message: Message,
    agent_message: maidan_a2a::A2aMessage,
}

async fn post_a2a_message(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<PostedA2a, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(MESSAGE_POST) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let req: SendMessageRequest = serde_json::from_value(params).map_err(|e| {
        JsonRpcResponse::error(
            id.clone(),
            ERR_PARAMS,
            format!("invalid SendMessage params: {e}"),
        )
    })?;
    let ctx = maidan_context_from_metadata(&req.metadata)
        .map_err(|msg| JsonRpcResponse::error(id.clone(), ERR_PARAMS, msg))?;
    let body_text = message_text(&req.message).ok_or_else(|| {
        JsonRpcResponse::error(id.clone(), ERR_PARAMS, "message must include a text part")
    })?;
    let thread_ctx = resolve_thread_context(state.store.as_ref(), ThreadId(ctx.thread_id))
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_PARAMS, e.to_string()))?;
    if let Err(e) = auth.ensure_workspace(thread_ctx.workspace_id) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    // Cluster 179: enforce per-channel access on the A2A ingress — the RBAC arc
    // (160–165) gated REST/MCP/WS/references but not this external surface, so a
    // `message:post` token could post into a private channel it isn't a member
    // of. Mirror the REST thread-route check.
    if let Err(e) =
        maidan_auth::ensure_channel_access(state.store.as_ref(), auth, thread_ctx.channel_id).await
    {
        return Err(JsonRpcResponse::error(id.clone(), -32001, e.to_string()));
    }
    let posted = state
        .store
        .post_message(NewMessage {
            thread_id: ThreadId(ctx.thread_id),
            author_id: MemberId(ctx.author_id),
            body: body_text,
            metadata: serde_json::json!({ "a2a": true }),
            content: None,
        })
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    publish(
        state,
        Event::MessagePosted {
            occurred_at: Utc::now(),
            workspace_id: thread_ctx.workspace_id,
            channel_id: thread_ctx.channel_id,
            thread_id: ThreadId(ctx.thread_id),
            dm_conversation_id: None,
            message: posted.clone(),
        },
    )
    .await;
    Ok(PostedA2a {
        task_id: uuid::Uuid::new_v4().to_string(),
        workspace_id: thread_ctx.workspace_id,
        thread_id: ctx.thread_id,
        message: posted,
        agent_message: req.message,
    })
}

async fn dispatch_send_message(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    let posted = post_a2a_message(state, auth, id.clone(), params).await?;
    let task = Task {
        id: posted.task_id.clone(),
        context_id: Some(posted.thread_id.to_string()),
        status: TaskStatus {
            state: TASK_STATE_COMPLETED.to_string(),
            message: Some(posted.agent_message),
        },
        metadata: Some(serde_json::json!({
            "maidan": { "messageId": posted.message.id.0 }
        })),
    };
    persist_task(state, posted.workspace_id, &task).await;
    let result = SendMessageResponse { task };
    let value = serde_json::to_value(result)
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
}

async fn dispatch_send_streaming_message(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Response {
    let posted = match post_a2a_message(state, auth, id.clone(), params).await {
        Ok(p) => p,
        Err(resp) => return Json(resp).into_response(),
    };

    let working = Task {
        id: posted.task_id.clone(),
        context_id: Some(posted.thread_id.to_string()),
        status: TaskStatus {
            state: TASK_STATE_WORKING.to_string(),
            message: None,
        },
        metadata: Some(serde_json::json!({
            "maidan": { "messageId": posted.message.id.0 }
        })),
    };
    persist_task(state, posted.workspace_id, &working).await;

    let completed = Task {
        id: posted.task_id.clone(),
        context_id: Some(posted.thread_id.to_string()),
        status: TaskStatus {
            state: TASK_STATE_COMPLETED.to_string(),
            message: Some(posted.agent_message),
        },
        metadata: working.metadata.clone(),
    };
    persist_task(state, posted.workspace_id, &completed).await;

    let status_update = TaskStatusUpdateEvent {
        task_id: posted.task_id,
        context_id: Some(posted.thread_id.to_string()),
        status: completed.status.clone(),
        is_final: true,
    };

    let frames = [
        JsonRpcResponse::success(
            id.clone(),
            serde_json::to_value(StreamResponseTask { task: working })
                .unwrap_or(serde_json::Value::Null),
        ),
        JsonRpcResponse::success(
            id,
            serde_json::to_value(StreamResponseStatusUpdate { status_update })
                .unwrap_or(serde_json::Value::Null),
        ),
    ];

    let stream = futures::stream::iter(frames).map(|frame| {
        let data = serde_json::to_string(&frame).unwrap_or_default();
        Ok::<SseEvent, Infallible>(SseEvent::default().data(data))
    });
    Sse::new(stream).into_response()
}

async fn dispatch_get_task(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(MESSAGE_POST) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let req: GetTaskRequest = serde_json::from_value(params).map_err(|e| {
        JsonRpcResponse::error(
            id.clone(),
            ERR_PARAMS,
            format!("invalid GetTask params: {e}"),
        )
    })?;
    let task = load_task(state, &req.id)
        .await
        .map_err(|_| JsonRpcResponse::error(id.clone(), ERR_PARAMS, "task not found"))?;
    ensure_task_workspace_access(state, auth, &id, &req.id, &task).await?;
    let value = serde_json::to_value(task)
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
}

async fn dispatch_set_push_config(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(WORKSPACE_WRITE) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let req: SetPushNotificationConfigRequest = serde_json::from_value(params).map_err(|e| {
        JsonRpcResponse::error(
            id.clone(),
            ERR_PARAMS,
            format!("invalid push config params: {e}"),
        )
    })?;
    if req.url.trim().is_empty() {
        return Err(JsonRpcResponse::error(id, ERR_PARAMS, "url is required"));
    }
    if auth.bypass {
        return Err(JsonRpcResponse::error(
            id,
            ERR_PARAMS,
            "push config requires a workspace-scoped bearer token",
        ));
    }
    state
        .store
        .upsert_a2a_push_config(auth.workspace_id, &req.url)
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(
        id,
        serde_json::json!({ "ok": true }),
    ))
}

async fn dispatch_get_push_config(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(WORKSPACE_WRITE) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    if auth.bypass {
        return Err(JsonRpcResponse::error(
            id,
            ERR_PARAMS,
            "push config requires a workspace-scoped bearer token",
        ));
    }
    let url = state
        .store
        .get_a2a_push_config(auth.workspace_id)
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    let resp = GetPushNotificationConfigResponse {
        config: url.map(|url| maidan_a2a::PushNotificationConfig { url }),
    };
    let value = serde_json::to_value(resp)
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
}

async fn dispatch_tasks_cancel(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(MESSAGE_POST) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let req: GetTaskRequest = serde_json::from_value(params).map_err(|e| {
        JsonRpcResponse::error(
            id.clone(),
            ERR_PARAMS,
            format!("invalid tasks/cancel params: {e}"),
        )
    })?;
    let mut task = load_task(state, &req.id)
        .await
        .map_err(|_| JsonRpcResponse::error(id.clone(), ERR_PARAMS, "task not found"))?;
    let workspace_id = ensure_task_workspace_access(state, auth, &id, &req.id, &task).await?;
    if is_terminal_task_state(&task.status.state) {
        return Err(JsonRpcResponse::error(
            id,
            ERR_UNSUPPORTED,
            "task is in a terminal state",
        ));
    }
    task.status.state = TASK_STATE_CANCELED.to_string();
    task.status.message = None;
    if let Some(ws) = workspace_id {
        persist_task(state, ws, &task).await;
    }
    let value = serde_json::to_value(task)
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
}

async fn dispatch_subscribe_to_task(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Response {
    if let Err(e) = auth.require_capability(MESSAGE_POST) {
        return Json(JsonRpcResponse::error(id, -32001, e.to_string())).into_response();
    }
    let req: GetTaskRequest = match serde_json::from_value(params) {
        Ok(r) => r,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                id.clone(),
                ERR_PARAMS,
                format!("invalid SubscribeToTask params: {e}"),
            ))
            .into_response();
        }
    };
    let task = match load_task(state, &req.id).await {
        Ok(t) => t,
        Err(_) => {
            return Json(JsonRpcResponse::error(id, ERR_PARAMS, "task not found")).into_response();
        }
    };
    if let Err(resp) = ensure_task_workspace_access(state, auth, &id, &req.id, &task).await {
        return Json(resp).into_response();
    }
    if is_terminal_task_state(&task.status.state) {
        return Json(JsonRpcResponse::error(
            id,
            ERR_UNSUPPORTED,
            "task is in a terminal state",
        ))
        .into_response();
    }

    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let state_bg = state.clone();
    let task_id = req.id.clone();
    let rpc_id = id.clone();
    let initial = task.clone();
    tokio::spawn(async move {
        let first = JsonRpcResponse::success(
            rpc_id.clone(),
            serde_json::to_value(StreamResponseTask {
                task: initial.clone(),
            })
            .unwrap_or_default(),
        );
        if tx.send(first).await.is_err() {
            return;
        }
        let mut last_state = initial.status.state.clone();
        for _ in 0..SUBSCRIBE_MAX_POLLS {
            if is_terminal_task_state(&last_state) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(SUBSCRIBE_POLL_MS)).await;
            let current = match load_task(&state_bg, &task_id).await {
                Ok(current) => current,
                Err(err) => {
                    tracing::warn!(task_id, error = %err, "a2a subscribe poll: load_task failed, ending stream");
                    return;
                }
            };
            if current.status.state == last_state {
                continue;
            }
            last_state = current.status.state.clone();
            let update = TaskStatusUpdateEvent {
                task_id: task_id.clone(),
                context_id: current.context_id.clone(),
                status: current.status.clone(),
                is_final: is_terminal_task_state(&last_state),
            };
            let frame = JsonRpcResponse::success(
                rpc_id.clone(),
                serde_json::to_value(StreamResponseStatusUpdate {
                    status_update: update,
                })
                .unwrap_or_default(),
            );
            if tx.send(frame).await.is_err() {
                return;
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|frame| Ok::<SseEvent, Infallible>(sse_json_rpc_frame(frame)));
    Sse::new(stream).into_response()
}

#[derive(Debug, Serialize)]
pub struct AgentCard {
    pub name: String,
    pub version: String,
    pub protocol_version: String,
    pub rpc_url: String,
    pub ingress_url: String,
    pub capabilities: Vec<String>,
}

pub async fn agent_card() -> impl IntoResponse {
    Json(AgentCard {
        name: "maidan".into(),
        version: crate::version().to_string(),
        protocol_version: "1.0".into(),
        rpc_url: "/a2a/v1/rpc".into(),
        ingress_url: "/a2a/v1/events".into(),
        capabilities: vec![
            METHOD_SEND_MESSAGE.into(),
            METHOD_SEND_STREAMING_MESSAGE.into(),
            METHOD_GET_TASK.into(),
            METHOD_SET_PUSH_NOTIFICATION_CONFIG.into(),
            METHOD_GET_PUSH_NOTIFICATION_CONFIG.into(),
            METHOD_SUBSCRIBE_TO_TASK.into(),
            METHOD_TASKS_RESUBSCRIBE.into(),
            METHOD_TASKS_CANCEL.into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::deliver_a2a_push;
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    };

    use axum::{http::StatusCode, routing::post, Router};

    /// Spawn a push endpoint that returns 500 for the first `fail_n` hits, then
    /// 200. Returns (base_url, hit_counter).
    async fn push_server(fail_n: u32) -> (String, Arc<AtomicU32>) {
        let hits = Arc::new(AtomicU32::new(0));
        let hits_route = hits.clone();
        let app = Router::new().route(
            "/push",
            post(move || {
                let hits = hits_route.clone();
                async move {
                    let n = hits.fetch_add(1, Ordering::SeqCst);
                    if n < fail_n {
                        StatusCode::INTERNAL_SERVER_ERROR
                    } else {
                        StatusCode::OK
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/push"), hits)
    }

    #[tokio::test]
    async fn a2a_push_retries_then_succeeds() {
        // Fails twice, succeeds on the third attempt.
        let (url, hits) = push_server(2).await;
        deliver_a2a_push(&url, &serde_json::json!({"task": "t1"}), "t1").await;
        assert_eq!(hits.load(Ordering::SeqCst), 3, "should retry up to success");
    }

    #[tokio::test]
    async fn a2a_push_gives_up_after_max_attempts() {
        // Always fails — bounded at MAX_ATTEMPTS (3), then gives up (no hang/loop).
        let (url, hits) = push_server(u32::MAX).await;
        deliver_a2a_push(&url, &serde_json::json!({"task": "t2"}), "t2").await;
        assert_eq!(hits.load(Ordering::SeqCst), 3, "should cap at MAX_ATTEMPTS");
    }
}
