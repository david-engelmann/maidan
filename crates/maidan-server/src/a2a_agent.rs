//! Google A2A protocol v1.0 JSON-RPC ingress (`POST /a2a/v1/rpc`).

use std::convert::Infallible;
use std::time::Duration;

use std::collections::HashMap;

use axum::http::StatusCode;
use axum::response::sse::Event as SseEvent;
use axum::response::{IntoResponse, Response, Sse};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::StreamExt;
use maidan_a2a::{
    is_terminal_task_state, maidan_context_from_metadata, message_content,
    message_parts_from_content, message_text, A2aMessage, DeleteTaskPushNotificationConfigRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, JsonRpcId, JsonRpcRequest,
    JsonRpcResponse, ListTaskPushNotificationConfigsRequest,
    ListTaskPushNotificationConfigsResponse, ListTasksRequest, ListTasksResponse,
    SendMessageRequest, SendMessageResponse, StreamResponseStatusUpdate, StreamResponseTask, Task,
    TaskPushNotificationConfig, TaskStatus, TaskStatusUpdateEvent, TextPart, METHOD_CANCEL_TASK,
    METHOD_CREATE_PUSH_NOTIFICATION_CONFIG, METHOD_DELETE_PUSH_NOTIFICATION_CONFIG,
    METHOD_GET_EXTENDED_AGENT_CARD, METHOD_GET_PUSH_NOTIFICATION_CONFIG, METHOD_GET_TASK,
    METHOD_LIST_PUSH_NOTIFICATION_CONFIGS, METHOD_LIST_TASKS, METHOD_SEND_MESSAGE,
    METHOD_SEND_STREAMING_MESSAGE, METHOD_SUBSCRIBE_TO_TASK, TASK_STATE_CANCELED,
    TASK_STATE_COMPLETED, TASK_STATE_WORKING,
};
use maidan_auth::capability::MESSAGE_POST;
use maidan_auth::capability::WORKSPACE_WRITE;
use maidan_auth::AuthContext;
use maidan_router::resolve_thread_context;
use maidan_types::*;
use serde::Serialize;
use uuid::Uuid;

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
        METHOD_LIST_TASKS => {
            json_response(dispatch_list_tasks(&state, &auth, id, body.params).await)
        }
        METHOD_GET_EXTENDED_AGENT_CARD => {
            json_response(dispatch_get_extended_agent_card(&auth, id).await)
        }
        METHOD_CANCEL_TASK => {
            json_response(dispatch_tasks_cancel(&state, &auth, id, body.params).await)
        }
        METHOD_CREATE_PUSH_NOTIFICATION_CONFIG => {
            json_response(dispatch_create_push_config(&state, &auth, id, body.params).await)
        }
        METHOD_GET_PUSH_NOTIFICATION_CONFIG => {
            json_response(dispatch_get_push_config(&state, &auth, id, body.params).await)
        }
        METHOD_LIST_PUSH_NOTIFICATION_CONFIGS => {
            json_response(dispatch_list_push_configs(&state, &auth, id, body.params).await)
        }
        METHOD_DELETE_PUSH_NOTIFICATION_CONFIG => {
            json_response(dispatch_delete_push_config(&state, &auth, id, body.params).await)
        }
        METHOD_SUBSCRIBE_TO_TASK => {
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
    if let Ok(configs) = state.store.list_a2a_task_push_configs(&task.id).await {
        for (_config_id, url) in configs {
            let task_id = task.id.clone();
            let value = value.clone();
            tokio::spawn(async move {
                deliver_a2a_push(&url, &value, &task_id).await;
            });
        }
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
                if let Err(e) = maidan_auth::ensure_thread_access(
                    state.store.as_ref(),
                    auth,
                    ThreadId(thread_id),
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
    // Preserve the A2A message's parts as structured content (Cluster 194); A2A
    // ingest previously dropped them (`content: None`), unlike REST/MCP posts.
    let content = message_content(&req.message);
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
        maidan_auth::ensure_thread_access(state.store.as_ref(), auth, ThreadId(ctx.thread_id)).await
    {
        return Err(JsonRpcResponse::error(id.clone(), -32001, e.to_string()));
    }
    let (posted, stored) = state
        .store
        .post_message_with_event(
            NewMessage {
                thread_id: ThreadId(ctx.thread_id),
                author_id: MemberId(ctx.author_id),
                body: body_text,
                metadata: serde_json::json!({ "a2a": true }),
                content,
            },
            None,
        )
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    crate::routes::publish_stored(state, stored).await;
    // Egress (Cluster 267): render the outbound A2A message from the STORED message's
    // canonical content (content → parts), not a raw echo of the request — so an A2A
    // consumer sees Maidan's stored representation. Falls back to the body when there
    // are no content blocks. Faithful round-trip for A2A-ingested (text) messages.
    let out_parts = message_parts_from_content(posted.content.as_deref().unwrap_or(&[]));
    let out_parts = if out_parts.is_empty() {
        vec![TextPart {
            kind: "text".to_string(),
            text: posted.body.clone(),
        }]
    } else {
        out_parts
    };
    let agent_message = A2aMessage {
        role: req.message.role.clone(),
        parts: out_parts,
        metadata: req.message.metadata.clone(),
    };
    Ok(PostedA2a {
        task_id: uuid::Uuid::new_v4().to_string(),
        workspace_id: thread_ctx.workspace_id,
        thread_id: ctx.thread_id,
        message: posted,
        agent_message,
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

/// A2A `ListTasks`: the authenticated workspace's tasks, most-recently-updated
/// first, filtered by optional `contextId` and by per-channel access (a task whose
/// context thread the caller cannot read is dropped). Single-page for now
/// (`nextPageToken` always empty); `pageSize` defaults to 50, clamped to 1..=200.
async fn dispatch_list_tasks(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(MESSAGE_POST) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let req: ListTasksRequest = if params.is_null() {
        ListTasksRequest::default()
    } else {
        serde_json::from_value(params).map_err(|e| {
            JsonRpcResponse::error(
                id.clone(),
                ERR_PARAMS,
                format!("invalid ListTasks params: {e}"),
            )
        })?
    };
    let page_size = req.page_size.unwrap_or(50).clamp(1, 200);
    let raw = state
        .store
        .list_a2a_tasks(auth.workspace_id, page_size as i64)
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    let mut tasks: Vec<Task> = Vec::new();
    for value in raw {
        let Ok(task) = serde_json::from_value::<Task>(value) else {
            continue;
        };
        if let Some(ctx) = &req.context_id {
            if task.context_id.as_deref() != Some(ctx.as_str()) {
                continue;
            }
        }
        // Per-channel RBAC: drop tasks whose context thread the caller can't read.
        if let Some(context_id) = &task.context_id {
            if let Ok(thread_id) = uuid::Uuid::parse_str(context_id) {
                match maidan_auth::can_access_thread(
                    state.store.as_ref(),
                    auth,
                    ThreadId(thread_id),
                )
                .await
                {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(e) => {
                        return Err(JsonRpcResponse::error(
                            id.clone(),
                            ERR_INTERNAL,
                            e.to_string(),
                        ))
                    }
                }
            }
        }
        tasks.push(task);
    }
    let total_size = tasks.len() as i32;
    let resp = ListTasksResponse {
        tasks,
        next_page_token: String::new(),
        page_size,
        total_size,
    };
    let value = serde_json::to_value(resp)
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
}

/// A2A `GetExtendedAgentCard`: the Agent Card for an authenticated client. Same
/// content as the public card today; §4.4.1 schema conformance lands in a later
/// arc cluster.
async fn dispatch_get_extended_agent_card(
    auth: &AuthContext,
    id: JsonRpcId,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(MESSAGE_POST) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let value = serde_json::to_value(agent_card_payload())
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
}

async fn dispatch_create_push_config(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(WORKSPACE_WRITE) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let mut req: TaskPushNotificationConfig = serde_json::from_value(params).map_err(|e| {
        JsonRpcResponse::error(
            id.clone(),
            ERR_PARAMS,
            format!("invalid CreateTaskPushNotificationConfig params: {e}"),
        )
    })?;
    if req.task_id.trim().is_empty() {
        return Err(JsonRpcResponse::error(id, ERR_PARAMS, "taskId is required"));
    }
    if req.url.trim().is_empty() {
        return Err(JsonRpcResponse::error(id, ERR_PARAMS, "url is required"));
    }
    let task = load_task(state, &req.task_id)
        .await
        .map_err(|_| JsonRpcResponse::error(id.clone(), ERR_PARAMS, "task not found"))?;
    ensure_task_workspace_access(state, auth, &id, &req.task_id, &task).await?;
    let config_id = req
        .config_id
        .clone()
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    state
        .store
        .create_a2a_task_push_config(&req.task_id, &config_id, &req.url)
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    req.config_id = Some(config_id);
    let value = serde_json::to_value(req)
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
}

async fn dispatch_get_push_config(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(WORKSPACE_WRITE) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let req: GetTaskPushNotificationConfigRequest =
        serde_json::from_value(params).map_err(|e| {
            JsonRpcResponse::error(
                id.clone(),
                ERR_PARAMS,
                format!("invalid GetTaskPushNotificationConfig params: {e}"),
            )
        })?;
    let task = load_task(state, &req.task_id)
        .await
        .map_err(|_| JsonRpcResponse::error(id.clone(), ERR_PARAMS, "task not found"))?;
    ensure_task_workspace_access(state, auth, &id, &req.task_id, &task).await?;
    let url = state
        .store
        .get_a2a_task_push_config(&req.task_id, &req.id)
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    let Some(url) = url else {
        return Err(JsonRpcResponse::error(
            id,
            ERR_PARAMS,
            "push config not found",
        ));
    };
    let cfg = TaskPushNotificationConfig {
        config_id: Some(req.id),
        task_id: req.task_id,
        url,
    };
    let value = serde_json::to_value(cfg)
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
}

async fn dispatch_list_push_configs(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(WORKSPACE_WRITE) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let req: ListTaskPushNotificationConfigsRequest =
        serde_json::from_value(params).map_err(|e| {
            JsonRpcResponse::error(
                id.clone(),
                ERR_PARAMS,
                format!("invalid ListTaskPushNotificationConfigs params: {e}"),
            )
        })?;
    let task = load_task(state, &req.task_id)
        .await
        .map_err(|_| JsonRpcResponse::error(id.clone(), ERR_PARAMS, "task not found"))?;
    ensure_task_workspace_access(state, auth, &id, &req.task_id, &task).await?;
    let rows = state
        .store
        .list_a2a_task_push_configs(&req.task_id)
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    let configs = rows
        .into_iter()
        .map(|(config_id, url)| TaskPushNotificationConfig {
            config_id: Some(config_id),
            task_id: req.task_id.clone(),
            url,
        })
        .collect();
    let resp = ListTaskPushNotificationConfigsResponse {
        configs,
        next_page_token: String::new(),
    };
    let value = serde_json::to_value(resp)
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
}

async fn dispatch_delete_push_config(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
    if let Err(e) = auth.require_capability(WORKSPACE_WRITE) {
        return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
    }
    let req: DeleteTaskPushNotificationConfigRequest =
        serde_json::from_value(params).map_err(|e| {
            JsonRpcResponse::error(
                id.clone(),
                ERR_PARAMS,
                format!("invalid DeleteTaskPushNotificationConfig params: {e}"),
            )
        })?;
    let task = load_task(state, &req.task_id)
        .await
        .map_err(|_| JsonRpcResponse::error(id.clone(), ERR_PARAMS, "task not found"))?;
    ensure_task_workspace_access(state, auth, &id, &req.task_id, &task).await?;
    let removed = state
        .store
        .delete_a2a_task_push_config(&req.task_id, &req.id)
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    if !removed {
        return Err(JsonRpcResponse::error(
            id,
            ERR_PARAMS,
            "push config not found",
        ));
    }
    Ok(JsonRpcResponse::success(id, serde_json::json!({})))
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
            format!("invalid CancelTask params: {e}"),
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

// ===== HTTP+JSON/REST binding (§11) — thin adapters over the JSON-RPC ops =====
// Each REST route builds the operation's params from the path/query/body and calls
// the shared `dispatch_*` handler, then maps the JSON-RPC result/error to HTTP.
// Streaming ops (message:stream, tasks:subscribe) are deferred.

/// A dummy JSON-RPC id for the REST binding (the ops require one; REST discards it).
fn rest_id() -> JsonRpcId {
    JsonRpcId::Number(0)
}

/// Map an operation result to a REST HTTP response. `result` → 200; error → an
/// HTTP status for Maidan's JSON-RPC code. NOTE Maidan overloads `-32001` for
/// auth/capability failures, so it maps to 403 (not the spec's 404 TaskNotFound).
fn rest_response(result: Result<JsonRpcResponse, JsonRpcResponse>) -> Response {
    let resp = match result {
        Ok(r) => r,
        Err(r) => r,
    };
    if let Some(value) = resp.result {
        return (StatusCode::OK, Json(value)).into_response();
    }
    let (code, message) = resp
        .error
        .map(|e| (e.code, e.message))
        .unwrap_or((ERR_INTERNAL, "internal error".to_string()));
    let status = match code {
        -32001 => StatusCode::FORBIDDEN,
        ERR_PARAMS => StatusCode::BAD_REQUEST,
        ERR_METHOD => StatusCode::NOT_FOUND,
        ERR_INTERNAL => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (
        status,
        Json(serde_json::json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

pub async fn rest_send_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    rest_response(dispatch_send_message(&state, &auth, rest_id(), body).await)
}

pub async fn rest_list_tasks(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let mut params = serde_json::Map::new();
    if let Some(ctx) = q.get("contextId") {
        params.insert("contextId".into(), serde_json::Value::String(ctx.clone()));
    }
    if let Some(ps) = q.get("pageSize").and_then(|v| v.parse::<i64>().ok()) {
        params.insert("pageSize".into(), serde_json::Value::from(ps));
    }
    rest_response(dispatch_list_tasks(&state, &auth, rest_id(), params.into()).await)
}

pub async fn rest_get_task(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Response {
    let params = serde_json::json!({ "id": id });
    rest_response(dispatch_get_task(&state, &auth, rest_id(), params).await)
}

/// `POST /a2a/v1/tasks/{id}:cancel` — axum can't capture a partial segment, so the
/// whole `{id}:cancel` segment is captured and split on ':' (task ids are UUIDs, no
/// colon). Only the request/response `:cancel` op is handled; `:subscribe` (SSE) is
/// deferred.
pub async fn rest_task_custom_method(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(task_action): Path<String>,
) -> Response {
    let Some((id, action)) = task_action.rsplit_once(':') else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": { "message": "unknown task method" } })),
        )
            .into_response();
    };
    match action {
        "cancel" => {
            let params = serde_json::json!({ "id": id });
            rest_response(dispatch_tasks_cancel(&state, &auth, rest_id(), params).await)
        }
        other => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": { "message": format!("unsupported task method: {other}") } })),
        )
            .into_response(),
    }
}

pub async fn rest_create_push_config(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
    Json(mut body): Json<serde_json::Value>,
) -> Response {
    if let Some(obj) = body.as_object_mut() {
        obj.insert("taskId".into(), serde_json::Value::String(id));
    }
    rest_response(dispatch_create_push_config(&state, &auth, rest_id(), body).await)
}

pub async fn rest_get_push_config(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, config_id)): Path<(String, String)>,
) -> Response {
    let params = serde_json::json!({ "taskId": id, "id": config_id });
    rest_response(dispatch_get_push_config(&state, &auth, rest_id(), params).await)
}

pub async fn rest_list_push_configs(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Response {
    let params = serde_json::json!({ "taskId": id });
    rest_response(dispatch_list_push_configs(&state, &auth, rest_id(), params).await)
}

pub async fn rest_delete_push_config(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, config_id)): Path<(String, String)>,
) -> Response {
    let params = serde_json::json!({ "taskId": id, "id": config_id });
    rest_response(dispatch_delete_push_config(&state, &auth, rest_id(), params).await)
}

pub async fn rest_extended_agent_card(Extension(auth): Extension<AuthContext>) -> Response {
    rest_response(dispatch_get_extended_agent_card(&auth, rest_id()).await)
}

/// The A2A protocol version Maidan's interfaces expose (spec `AgentInterface`
/// `protocolVersion`; the proto examples use "0.3"/"1.0").
const A2A_PROTOCOL_VERSION: &str = "1.0";

/// One transport binding advertised in the Agent Card's `supportedInterfaces`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInterface {
    pub url: String,
    pub protocol_binding: String,
    pub protocol_version: String,
}

/// A2A `AgentCapabilities` (spec §4.4.1).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
    pub extended_agent_card: bool,
}

/// A2A `AgentProvider`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    pub url: String,
    pub organization: String,
}

/// A2A `AgentSkill`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
}

/// A2A v1.0 Agent Card (spec §4.4.1 shape). Served at
/// `/.well-known/agent-card.json` and returned by `GetExtendedAgentCard`. The
/// interface URLs are host-relative; a deployment behind a fixed public origin
/// can front them with an absolute HTTPS URL (see the transport-negotiation work).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub supported_interfaces: Vec<AgentInterface>,
    pub provider: AgentProvider,
    pub version: String,
    pub capabilities: AgentCapabilities,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: Vec<AgentSkill>,
}

fn agent_card_payload() -> AgentCard {
    AgentCard {
        name: "maidan".into(),
        description:
            "Maidan — the operating layer for teams of AI agents. Exposes A2A tasks over a \
             shared, durable workspace."
                .into(),
        supported_interfaces: vec![
            AgentInterface {
                url: "/a2a/v1/rpc".into(),
                protocol_binding: "JSONRPC".into(),
                protocol_version: A2A_PROTOCOL_VERSION.into(),
            },
            AgentInterface {
                url: "/a2a/v1".into(),
                protocol_binding: "HTTP+JSON".into(),
                protocol_version: A2A_PROTOCOL_VERSION.into(),
            },
        ],
        provider: AgentProvider {
            url: "https://github.com/david-engelmann/maidan".into(),
            organization: "Maidan".into(),
        },
        version: crate::version().to_string(),
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: true,
            extended_agent_card: true,
        },
        default_input_modes: vec!["text/plain".into()],
        default_output_modes: vec!["text/plain".into()],
        skills: vec![AgentSkill {
            id: "collaborate".into(),
            name: "Workspace collaboration".into(),
            description:
                "Post and read messages in shared threads, and drive long-running A2A tasks \
                 (send/get/list/cancel, subscribe, push configs) within a Maidan workspace."
                    .into(),
            tags: vec!["messaging".into(), "tasks".into(), "collaboration".into()],
        }],
    }
}

pub async fn agent_card() -> impl IntoResponse {
    Json(agent_card_payload())
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
