//! FSM automation hook registration and dispatch on thread transitions.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    decrypt_peer_secret_rotating, encrypt_peer_secret, AuthContext, TokenSecret,
};
use maidan_mcp::tools;
use maidan_types::{
    ChannelId, FsmHook, FsmHookId, FsmHookWithSecret, MemberId, NewFsmHook, SlashHandlerKind,
    Thread, ThreadId, ThreadState, WorkspaceId,
};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};

use crate::dto::{CreateFsmHook, FsmHookResponse, MintFsmHookResponse};
use crate::error::{ApiError, ApiJson};
use crate::state::{AppState, FsmHookRuntime};

type ApiResult<T> = Result<T, ApiError>;

const DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

pub fn remember_fsm_secret(
    secrets: &Arc<RwLock<HashMap<FsmHookId, String>>>,
    id: FsmHookId,
    secret: String,
) {
    if let Ok(mut guard) = secrets.write() {
        guard.insert(id, secret);
    }
}

pub fn forget_fsm_secret(secrets: &Arc<RwLock<HashMap<FsmHookId, String>>>, id: FsmHookId) {
    if let Ok(mut guard) = secrets.write() {
        guard.remove(&id);
    }
}

pub fn resolve_fsm_secret(
    runtime: &FsmHookRuntime,
    hook_id: FsmHookId,
    secret_ciphertext: &str,
) -> Option<String> {
    if secret_ciphertext.is_empty() {
        return None;
    }
    if let Ok(guard) = runtime.secrets.read() {
        if let Some(secret) = guard.get(&hook_id) {
            return Some(secret.clone());
        }
    }
    let key = runtime.encryption_key.as_deref()?;
    let secret = decrypt_peer_secret_rotating(secret_ciphertext, key).ok()?;
    remember_fsm_secret(&runtime.secrets, hook_id, secret.clone());
    Some(secret)
}

fn parse_opt_state(s: Option<&str>) -> ApiResult<Option<ThreadState>> {
    match s {
        None => Ok(None),
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                ThreadState::parse(trimmed)
                    .ok_or_else(|| ApiError::BadRequest(format!("unknown thread state: {trimmed}")))
                    .map(Some)
            }
        }
    }
}

fn validate_http_target(url: &str) -> ApiResult<()> {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(ApiError::BadRequest(
            "handler_target url must use http or https".into(),
        ));
    }
    if trimmed.len() > 2048 || trimmed.as_bytes().contains(&b' ') {
        return Err(ApiError::BadRequest("invalid handler url".into()));
    }
    Ok(())
}

fn validate_mcp_target(tool: &str) -> ApiResult<()> {
    tools::required_capability(tool)
        .map_err(|_| ApiError::BadRequest(format!("unknown mcp tool for fsm hook: {tool}")))?;
    Ok(())
}

pub async fn create_fsm_hook(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateFsmHook>,
) -> ApiResult<(StatusCode, Json<MintFsmHookResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let from_state = parse_opt_state(body.from_state.as_deref())?;
    let to_state = parse_opt_state(body.to_state.as_deref())?;
    let handler_kind = SlashHandlerKind::parse(&body.handler_kind)
        .ok_or_else(|| ApiError::BadRequest("handler_kind must be http or mcp_tool".into()))?;
    match handler_kind {
        SlashHandlerKind::Http => validate_http_target(&body.handler_target)?,
        SlashHandlerKind::McpTool => validate_mcp_target(&body.handler_target)?,
    }

    let mut secret_plain: Option<String> = None;
    let secret_ciphertext = if handler_kind == SlashHandlerKind::Http {
        let secret = TokenSecret::generate();
        let key = state.fsm_hooks.encryption_key.as_deref().ok_or_else(|| {
            ApiError::Internal("FEDERATION_ENCRYPTION_KEY must be set for http fsm hooks".into())
        })?;
        let ciphertext = encrypt_peer_secret(secret.as_str(), key)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        secret_plain = Some(secret.as_str().to_string());
        ciphertext
    } else {
        String::new()
    };

    let hook = state
        .store
        .create_fsm_hook(NewFsmHook {
            workspace_id,
            label: body.label,
            from_state,
            to_state,
            handler_kind,
            handler_target: body.handler_target.trim().to_string(),
            secret_ciphertext,
        })
        .await?;

    if let Some(secret) = secret_plain {
        remember_fsm_secret(&state.fsm_hooks.secrets, hook.id, secret.clone());
        Ok((
            StatusCode::CREATED,
            Json(MintFsmHookResponse {
                hook: FsmHookResponse::from(hook),
                secret: Some(secret),
            }),
        ))
    } else {
        Ok((
            StatusCode::CREATED,
            Json(MintFsmHookResponse {
                hook: FsmHookResponse::from(hook),
                secret: None,
            }),
        ))
    }
}

pub async fn list_fsm_hooks(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<FsmHookResponse>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let hooks = state.store.list_fsm_hooks(workspace_id).await?;
    Ok(Json(hooks.into_iter().map(FsmHookResponse::from).collect()))
}

pub async fn revoke_fsm_hook(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, hook_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(workspace_id);
    let hook_id = FsmHookId(hook_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let hook = state.store.revoke_fsm_hook(hook_id).await?;
    if hook.workspace_id != workspace_id {
        return Err(ApiError::NotFound);
    }
    forget_fsm_secret(&state.fsm_hooks.secrets, hook_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Clone)]
struct FsmTransitionCtx {
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    actor_id: MemberId,
    from_state: ThreadState,
    to_state: ThreadState,
}

#[derive(Debug, Serialize)]
struct FsmHttpPayload {
    event: &'static str,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    actor_id: MemberId,
    from_state: ThreadState,
    to_state: ThreadState,
    thread: Thread,
}

pub async fn dispatch_thread_state_changed(
    state: &AppState,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    actor_id: MemberId,
    from_state: ThreadState,
    to_state: ThreadState,
    thread: Thread,
) {
    let hooks = match state
        .store
        .list_matching_fsm_hooks(workspace_id, from_state, to_state)
        .await
    {
        Ok(h) => h,
        Err(err) => {
            tracing::warn!(error = %err, "fsm hook list failed");
            return;
        }
    };
    if hooks.is_empty() {
        return;
    }
    let auth = AuthContext::bypass();
    let ctx = FsmTransitionCtx {
        workspace_id,
        channel_id,
        thread_id,
        actor_id,
        from_state,
        to_state,
    };
    for registration in hooks {
        let dispatch = async {
            match registration.hook.handler_kind {
                SlashHandlerKind::Http => {
                    dispatch_http(state, &registration, &ctx, thread.clone()).await
                }
                SlashHandlerKind::McpTool => {
                    dispatch_mcp_tool(state, &auth, &registration.hook, &ctx).await
                }
            }
        };
        match tokio::time::timeout(DISPATCH_TIMEOUT, dispatch).await {
            Ok(result) if result.get("ok").and_then(|v| v.as_bool()) == Some(true) => {
                tracing::debug!(
                    hook = %registration.hook.id,
                    from = %from_state.as_str(),
                    to = %to_state.as_str(),
                    "fsm hook dispatched"
                );
            }
            Ok(result) => {
                tracing::warn!(
                    hook = %registration.hook.id,
                    error = %result.get("error").unwrap_or(&JsonValue::Null),
                    "fsm hook dispatch failed"
                );
            }
            Err(_) => tracing::warn!(hook = %registration.hook.id, "fsm hook dispatch timed out"),
        }
    }
}

async fn dispatch_http(
    state: &AppState,
    registration: &FsmHookWithSecret,
    ctx: &FsmTransitionCtx,
    thread: Thread,
) -> JsonValue {
    let payload = FsmHttpPayload {
        event: "thread_state_changed",
        workspace_id: ctx.workspace_id,
        channel_id: ctx.channel_id,
        thread_id: ctx.thread_id,
        actor_id: ctx.actor_id,
        from_state: ctx.from_state,
        to_state: ctx.to_state,
        thread,
    };
    let body = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(err) => return json!({ "ok": false, "error": err.to_string() }),
    };
    match crate::automation_delivery::enqueue_fsm_http(
        state,
        ctx.workspace_id,
        registration.hook.id,
        &registration.hook.handler_target,
        &body,
    )
    .await
    {
        Ok(delivery_id) => json!({ "ok": true, "queued": true, "delivery_id": delivery_id }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

async fn dispatch_mcp_tool(
    state: &AppState,
    auth: &AuthContext,
    hook: &FsmHook,
    ctx: &FsmTransitionCtx,
) -> JsonValue {
    let args = json!({
        "workspace_id": ctx.workspace_id.0,
        "channel_id": ctx.channel_id.0,
        "thread_id": ctx.thread_id.0,
        "actor_id": ctx.actor_id.0,
        "from_state": ctx.from_state.as_str(),
        "to_state": ctx.to_state.as_str(),
    });
    match state.mcp.call_tool(auth, &hook.handler_target, &args).await {
        Ok(result) => json!({ "ok": true, "response": result }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    }
}
