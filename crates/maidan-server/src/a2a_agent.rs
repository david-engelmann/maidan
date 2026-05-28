//! Google A2A protocol v1.0 JSON-RPC ingress (`POST /a2a/v1/rpc`).

use axum::{extract::State, Extension, Json};
use chrono::Utc;
use maidan_a2a::{
    maidan_context_from_metadata, message_text, GetTaskRequest, JsonRpcId, JsonRpcRequest,
    JsonRpcResponse, SendMessageRequest, SendMessageResponse, Task, TaskStatus, METHOD_GET_TASK,
    METHOD_SEND_MESSAGE, TASK_STATE_COMPLETED,
};
use maidan_auth::capability::MESSAGE_POST;
use maidan_auth::AuthContext;
use maidan_router::resolve_thread_context;
use maidan_types::*;

use crate::routes::publish;
use crate::state::AppState;

const ERR_PARSE: i32 = -32700;
const ERR_METHOD: i32 = -32601;
const ERR_PARAMS: i32 = -32602;
const ERR_INTERNAL: i32 = -32603;

pub async fn json_rpc(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    if body.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            body.id,
            ERR_PARSE,
            "jsonrpc must be \"2.0\"",
        ));
    }
    let id = body.id.clone();
    let result = match body.method.as_str() {
        METHOD_SEND_MESSAGE => dispatch_send_message(&state, &auth, id.clone(), body.params).await,
        METHOD_GET_TASK => dispatch_get_task(&state, &auth, id.clone(), body.params).await,
        other => Ok(JsonRpcResponse::error(
            id,
            ERR_METHOD,
            format!("method not found: {other}"),
        )),
    };
    match result {
        Ok(resp) => Json(resp),
        Err(resp) => Json(resp),
    }
}

async fn dispatch_send_message(
    state: &AppState,
    auth: &AuthContext,
    id: JsonRpcId,
    params: serde_json::Value,
) -> Result<JsonRpcResponse, JsonRpcResponse> {
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
            message: posted.clone(),
        },
    )
    .await;
    let task_id = uuid::Uuid::new_v4().to_string();
    let task = Task {
        id: task_id.clone(),
        context_id: Some(ctx.thread_id.to_string()),
        status: TaskStatus {
            state: TASK_STATE_COMPLETED.to_string(),
            message: Some(req.message),
        },
        metadata: Some(serde_json::json!({
            "maidan": { "messageId": posted.id.0 }
        })),
    };
    state.a2a_tasks.insert(task.clone());
    let result = SendMessageResponse { task };
    let value = serde_json::to_value(result)
        .map_err(|e| JsonRpcResponse::error(id.clone(), ERR_INTERNAL, e.to_string()))?;
    Ok(JsonRpcResponse::success(id, value))
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
    let task = state
        .a2a_tasks
        .get(&req.id)
        .ok_or_else(|| JsonRpcResponse::error(id.clone(), ERR_PARAMS, "task not found"))?;
    if let Some(context_id) = &task.context_id {
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
