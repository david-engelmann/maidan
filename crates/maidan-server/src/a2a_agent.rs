//! Google A2A protocol v1.0 JSON-RPC ingress (`POST /a2a/v1/rpc`).

use std::convert::Infallible;

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
    METHOD_TASKS_RESUBSCRIBE, TASK_STATE_COMPLETED, TASK_STATE_WORKING,
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
        let url = url.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let _ = client
                .post(url)
                .header("Content-Type", "application/json")
                .json(&value)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await;
        });
    }
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
    let posted = state
        .store
        .post_message(NewMessage {
            thread_id: ThreadId(ctx.thread_id),
            author_id: MemberId(ctx.author_id),
            body: body_text,
            metadata: serde_json::json!({ "a2a": true }),
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
            serde_json::to_value(StreamResponseTask { task: working }).unwrap(),
        ),
        JsonRpcResponse::success(
            id,
            serde_json::to_value(StreamResponseStatusUpdate { status_update }).unwrap(),
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
    if let Some(ws) = state
        .store
        .get_a2a_task_workspace(&req.id)
        .await
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?
    {
        if let Err(e) = auth.ensure_workspace(ws) {
            return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
        }
    } else if let Some(context_id) = &task.context_id {
        if let Ok(thread_id) = uuid::Uuid::parse_str(context_id) {
            if let Ok(thread_ctx) =
                resolve_thread_context(state.store.as_ref(), ThreadId(thread_id)).await
            {
                if let Err(e) = auth.ensure_workspace(thread_ctx.workspace_id) {
                    return Err(JsonRpcResponse::error(id, -32001, e.to_string()));
                }
            }
        }
    }
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
    if let Ok(Some(ws)) = state.store.get_a2a_task_workspace(&req.id).await {
        if let Err(e) = auth.ensure_workspace(ws) {
            return Json(JsonRpcResponse::error(id, -32001, e.to_string())).into_response();
        }
    }
    if is_terminal_task_state(&task.status.state) {
        return Json(JsonRpcResponse::error(
            id,
            ERR_UNSUPPORTED,
            "task is in a terminal state",
        ))
        .into_response();
    }

    let frames = vec![JsonRpcResponse::success(
        id.clone(),
        serde_json::to_value(StreamResponseTask { task: task.clone() }).unwrap(),
    )];
    let stream = futures::stream::iter(frames).map(|frame| {
        let data = serde_json::to_string(&frame).unwrap_or_default();
        Ok::<SseEvent, Infallible>(SseEvent::default().data(data))
    });
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
        ],
    })
}
