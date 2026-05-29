//! Slash command registration and HTTP handler dispatch.

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
    decrypt_peer_secret, encrypt_peer_secret, AuthContext, TokenSecret,
};
use maidan_mcp::tools;
use maidan_router::ParsedSlashCommand;
use maidan_types::{
    ChannelId, MemberId, MessageId, NewSlashCommand, SlashCommand, SlashCommandId,
    SlashCommandWithSecret, SlashHandlerKind, ThreadId, WorkspaceId,
};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::dto::{CreateSlashCommand, MintSlashCommandResponse, SlashCommandResponse};
use crate::error::{ApiError, ApiJson};
use crate::state::{AppState, SlashRuntime};
use crate::webhooks::sign_payload;

type ApiResult<T> = Result<T, ApiError>;

const DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

pub fn remember_slash_secret(
    secrets: &Arc<RwLock<HashMap<SlashCommandId, String>>>,
    id: SlashCommandId,
    secret: String,
) {
    if let Ok(mut guard) = secrets.write() {
        guard.insert(id, secret);
    }
}

pub fn forget_slash_secret(
    secrets: &Arc<RwLock<HashMap<SlashCommandId, String>>>,
    id: SlashCommandId,
) {
    if let Ok(mut guard) = secrets.write() {
        guard.remove(&id);
    }
}

pub fn resolve_slash_secret(
    runtime: &SlashRuntime,
    command_id: SlashCommandId,
    secret_ciphertext: &str,
) -> Option<String> {
    if secret_ciphertext.is_empty() {
        return None;
    }
    if let Ok(guard) = runtime.secrets.read() {
        if let Some(secret) = guard.get(&command_id) {
            return Some(secret.clone());
        }
    }
    let key = runtime.encryption_key.as_deref()?;
    let secret = decrypt_peer_secret(secret_ciphertext, key).ok()?;
    remember_slash_secret(&runtime.secrets, command_id, secret.clone());
    Some(secret)
}

fn validate_command_name(name: &str) -> ApiResult<String> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    if normalized.is_empty() || normalized.len() > 32 {
        return Err(ApiError::BadRequest(
            "slash command name must be 1-32 characters".into(),
        ));
    }
    if !normalized
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(ApiError::BadRequest(
            "slash command name may only contain a-z, 0-9, _, -".into(),
        ));
    }
    Ok(normalized)
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
        .map_err(|_| ApiError::BadRequest(format!("unknown mcp tool for slash handler: {tool}")))?;
    Ok(())
}

pub async fn create_slash_command(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateSlashCommand>,
) -> ApiResult<(StatusCode, Json<MintSlashCommandResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let name = validate_command_name(&body.name)?;
    let handler_kind = SlashHandlerKind::parse(&body.handler_kind)
        .ok_or_else(|| ApiError::BadRequest("handler_kind must be http or mcp_tool".into()))?;
    match handler_kind {
        SlashHandlerKind::Http => validate_http_target(&body.handler_target)?,
        SlashHandlerKind::McpTool => validate_mcp_target(&body.handler_target)?,
    }

    let mut secret_plain: Option<String> = None;
    let secret_ciphertext = if handler_kind == SlashHandlerKind::Http {
        let secret = TokenSecret::generate();
        let key = state.slash.encryption_key.as_deref().ok_or_else(|| {
            ApiError::Internal(
                "FEDERATION_ENCRYPTION_KEY must be set for http slash handlers".into(),
            )
        })?;
        let ciphertext = encrypt_peer_secret(secret.as_str(), key)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        secret_plain = Some(secret.as_str().to_string());
        ciphertext
    } else {
        String::new()
    };

    let command = state
        .store
        .create_slash_command(NewSlashCommand {
            workspace_id,
            name,
            description: body.description,
            handler_kind,
            handler_target: body.handler_target.trim().to_string(),
            secret_ciphertext,
        })
        .await?;

    if let Some(secret) = secret_plain {
        remember_slash_secret(&state.slash.secrets, command.id, secret.clone());
        Ok((
            StatusCode::CREATED,
            Json(MintSlashCommandResponse {
                command: SlashCommandResponse::from(command),
                secret: Some(secret),
            }),
        ))
    } else {
        Ok((
            StatusCode::CREATED,
            Json(MintSlashCommandResponse {
                command: SlashCommandResponse::from(command),
                secret: None,
            }),
        ))
    }
}

pub async fn list_slash_commands(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<SlashCommandResponse>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let commands = state.store.list_slash_commands(workspace_id).await?;
    Ok(Json(
        commands
            .into_iter()
            .map(SlashCommandResponse::from)
            .collect(),
    ))
}

pub async fn revoke_slash_command(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, command_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(workspace_id);
    let command_id = SlashCommandId(command_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let command = state.store.revoke_slash_command(command_id).await?;
    if command.workspace_id != workspace_id {
        return Err(ApiError::NotFound);
    }
    forget_slash_secret(&state.slash.secrets, command_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
struct SlashHttpPayload<'a> {
    command: &'a str,
    name: &'a str,
    text: &'a str,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    author_id: MemberId,
    message_id: MessageId,
}

pub async fn dispatch_slash_command(
    state: &AppState,
    auth: &AuthContext,
    parsed: &ParsedSlashCommand,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    author_id: MemberId,
    message_id: MessageId,
) -> Value {
    let lookup = state
        .store
        .get_slash_command_by_name(workspace_id, &parsed.name)
        .await;
    let Ok(registration) = lookup else {
        return json!({ "ok": false, "error": "unknown_command" });
    };

    let dispatch = async {
        match registration.command.handler_kind {
            SlashHandlerKind::Http => {
                dispatch_http(
                    state,
                    &registration,
                    parsed,
                    workspace_id,
                    channel_id,
                    thread_id,
                    author_id,
                    message_id,
                )
                .await
            }
            SlashHandlerKind::McpTool => {
                dispatch_mcp_tool(
                    state,
                    auth,
                    &registration.command,
                    parsed,
                    workspace_id,
                    channel_id,
                    thread_id,
                    author_id,
                )
                .await
            }
        }
    };

    match tokio::time::timeout(DISPATCH_TIMEOUT, dispatch).await {
        Ok(result) => result,
        Err(_) => json!({ "ok": false, "error": "timeout" }),
    }
}

async fn dispatch_http(
    state: &AppState,
    registration: &SlashCommandWithSecret,
    parsed: &ParsedSlashCommand,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    author_id: MemberId,
    message_id: MessageId,
) -> Value {
    let Some(secret) = resolve_slash_secret(
        &state.slash,
        registration.command.id,
        &registration.secret_ciphertext,
    ) else {
        return json!({ "ok": false, "error": "missing_signing_secret" });
    };
    let payload = SlashHttpPayload {
        command: &format!("/{}", parsed.name),
        name: &parsed.name,
        text: &parsed.args,
        workspace_id,
        channel_id,
        thread_id,
        author_id,
        message_id,
    };
    let body = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(err) => return json!({ "ok": false, "error": err.to_string() }),
    };
    let signature = sign_payload(&secret, &body);
    let client = Client::new();
    let response = match client
        .post(&registration.command.handler_target)
        .header("Content-Type", "application/json")
        .header("X-Maidan-Signature", signature)
        .header("X-Maidan-Command", &parsed.name)
        .body(body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(err) => return json!({ "ok": false, "error": err.to_string() }),
    };
    if !response.status().is_success() {
        return json!({
            "ok": false,
            "error": format!("HTTP {}", response.status())
        });
    }
    match response.json::<Value>().await {
        Ok(v) => json!({ "ok": true, "response": v }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    }
}

async fn dispatch_mcp_tool(
    state: &AppState,
    auth: &AuthContext,
    command: &SlashCommand,
    parsed: &ParsedSlashCommand,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    author_id: MemberId,
) -> Value {
    let args = build_mcp_arguments(parsed, workspace_id, channel_id, thread_id, author_id);
    match state
        .mcp
        .call_tool(auth, &command.handler_target, &args)
        .await
    {
        Ok(result) => json!({ "ok": true, "response": result }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    }
}

fn build_mcp_arguments(
    parsed: &ParsedSlashCommand,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    author_id: MemberId,
) -> Value {
    let mut base = if parsed.args.trim_start().starts_with('{') {
        serde_json::from_str(&parsed.args).unwrap_or_else(|_| json!({ "text": parsed.args }))
    } else if parsed.args.is_empty() {
        json!({})
    } else {
        json!({ "text": parsed.args })
    };
    let Some(obj) = base.as_object_mut() else {
        return json!({
            "workspace_id": workspace_id.0,
            "channel_id": channel_id.0,
            "thread_id": thread_id.0,
            "author_id": author_id.0,
            "text": parsed.args
        });
    };
    obj.entry("workspace_id")
        .or_insert_with(|| json!(workspace_id.0));
    obj.entry("channel_id")
        .or_insert_with(|| json!(channel_id.0));
    obj.entry("thread_id").or_insert_with(|| json!(thread_id.0));
    obj.entry("author_id").or_insert_with(|| json!(author_id.0));
    base
}

pub fn slash_metadata(parsed: &ParsedSlashCommand, dispatch: &Value) -> Value {
    json!({
        "slash_command": {
            "name": parsed.name,
            "args": parsed.args
        },
        "slash_response": dispatch
    })
}

pub fn merge_metadata(mut base: Value, extra: Value) -> Value {
    if !base.is_object() {
        base = json!({});
    }
    if let (Some(base_obj), Some(extra_obj)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra_obj {
            base_obj.insert(key.clone(), value.clone());
        }
    }
    base
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SlashInvocationSummary {
    pub ok: bool,
}
