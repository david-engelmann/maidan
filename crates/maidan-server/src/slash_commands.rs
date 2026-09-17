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
    decrypt_peer_secret_rotating, encrypt_peer_secret, AuthContext, TokenSecret,
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
    let secret = decrypt_peer_secret_rotating(secret_ciphertext, key).ok()?;
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
    let handler_kind = SlashHandlerKind::parse(&body.handler_kind).ok_or_else(|| {
        ApiError::BadRequest("handler_kind must be http, mcp_tool, or wasi".into())
    })?;
    let handler_target = match handler_kind {
        SlashHandlerKind::Http => {
            validate_http_target(&body.handler_target)?;
            body.handler_target.trim().to_string()
        }
        SlashHandlerKind::McpTool => {
            validate_mcp_target(&body.handler_target)?;
            body.handler_target.trim().to_string()
        }
        // Not just a syntax check: the sha must name an artifact this workspace
        // owns, or the command would be registrable and permanently unrunnable.
        SlashHandlerKind::Wasi => maidan_auth::resolve_wasi_handler_target(
            state.store.as_ref(),
            &auth,
            workspace_id,
            &body.handler_target,
        )
        .await
        .map_err(|e| match e {
            maidan_auth::WasiTargetError::Invalid(msg) => ApiError::BadRequest(msg),
            // A store that could not answer is not a bad request.
            maidan_auth::WasiTargetError::Store(err) => ApiError::from(err),
        })?,
    };

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
            handler_target,
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
            SlashHandlerKind::Wasi => {
                crate::wasi_handler::dispatch_wasi(
                    state,
                    auth,
                    &registration.command,
                    parsed,
                    workspace_id,
                    channel_id,
                    thread_id,
                    author_id,
                    message_id,
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
        .body(body.clone())
        .send()
        .await
    {
        Ok(r) => r,
        Err(err) => {
            let delivery_id = crate::automation_delivery::enqueue_slash_http(
                state,
                workspace_id,
                registration.command.id,
                &registration.command.handler_target,
                &parsed.name,
                &body,
            )
            .await
            .ok();
            return json!({
                "ok": false,
                "error": err.to_string(),
                "delivery_id": delivery_id,
                "retrying": delivery_id.is_some()
            });
        }
    };
    if !response.status().is_success() {
        let err = format!("HTTP {}", response.status());
        let delivery_id = crate::automation_delivery::enqueue_slash_http(
            state,
            workspace_id,
            registration.command.id,
            &registration.command.handler_target,
            &parsed.name,
            &body,
        )
        .await
        .ok();
        return json!({
            "ok": false,
            "error": err,
            "delivery_id": delivery_id,
            "retrying": delivery_id.is_some()
        });
    }
    match response.json::<Value>().await {
        Ok(v) => json!({ "ok": true, "response": v }),
        Err(err) => {
            let delivery_id = crate::automation_delivery::enqueue_slash_http(
                state,
                workspace_id,
                registration.command.id,
                &registration.command.handler_target,
                &parsed.name,
                &body,
            )
            .await
            .ok();
            json!({
                "ok": false,
                "error": err.to_string(),
                "delivery_id": delivery_id,
                "retrying": delivery_id.is_some()
            })
        }
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
    let args = build_mcp_arguments(
        &command.handler_target,
        parsed,
        workspace_id,
        channel_id,
        thread_id,
        author_id,
    );
    match state
        .mcp
        .call_tool(auth, &command.handler_target, &args)
        .await
    {
        Ok(result) => json!({ "ok": true, "response": result }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    }
}

/// Build a tool call from a slash invocation, injecting **only the context the
/// tool declares**.
///
/// This used to inject `workspace_id`, `channel_id`, `thread_id` and
/// `author_id` into every call regardless, and relied on each tool silently
/// discarding what it did not declare. That stopped being true when the
/// argument structs began rejecting unknown fields — `/channels` →
/// `list_channels` started failing because it was handed three ids it never
/// asked for — and it was a poor thing to depend on in the first place: a
/// bridge that sprays arguments at a callee is indistinguishable from one that
/// is passing the wrong ones.
///
/// The catalog already publishes each tool's `inputSchema`, so the declared
/// property set is the honest filter. A tool with no schema entry (or an empty
/// one) gets no injected context rather than all of it — fail closed, since an
/// unknown tool is exactly the case where guessing is least safe.
fn build_mcp_arguments(
    tool: &str,
    parsed: &ParsedSlashCommand,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    author_id: MemberId,
) -> Value {
    let declared = tools::declared_arguments(tool).unwrap_or_default();
    let mut base = if parsed.args.trim_start().starts_with('{') {
        serde_json::from_str(&parsed.args).unwrap_or_else(|_| json!({ "text": parsed.args }))
    } else if parsed.args.is_empty() {
        json!({})
    } else {
        json!({ "text": parsed.args })
    };
    let Some(obj) = base.as_object_mut() else {
        return json!({});
    };
    for (key, value) in [
        ("workspace_id", json!(workspace_id.0)),
        ("channel_id", json!(channel_id.0)),
        ("thread_id", json!(thread_id.0)),
        ("author_id", json!(author_id.0)),
    ] {
        if declared.contains(key) {
            obj.entry(key).or_insert(value);
        }
    }
    // `text` is only meaningful to a tool that declares it; the caller's own
    // args are left alone either way.
    if !declared.contains("text") {
        obj.remove("text");
    }
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

/// The server-side [`maidan_mcp::SlashDispatcher`]: lets the MCP `post_message`
/// handler (in `maidan-mcp`) run registered slash commands, which live here.
/// Attached to the `McpServer` once at startup from `main.rs`, so the
/// `AppState` it holds and the `Arc<McpServer>` inside it form a deliberate
/// process-lifetime shared-state graph (never built in tests, which leave the
/// dispatcher unset).
pub struct ServerSlashDispatcher {
    state: AppState,
}

impl ServerSlashDispatcher {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait::async_trait]
impl maidan_mcp::SlashDispatcher for ServerSlashDispatcher {
    async fn dispatch(
        &self,
        auth: &AuthContext,
        parsed: &ParsedSlashCommand,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        author_id: MemberId,
        message_id: MessageId,
    ) -> Value {
        let result = dispatch_slash_command(
            &self.state,
            auth,
            parsed,
            workspace_id,
            channel_id,
            thread_id,
            author_id,
            message_id,
        )
        .await;
        slash_metadata(parsed, &result)
    }
}

#[cfg(test)]
mod mcp_argument_tests {
    use super::*;

    fn parsed(args: &str) -> ParsedSlashCommand {
        ParsedSlashCommand {
            name: "x".into(),
            args: args.into(),
        }
    }

    fn ids() -> (WorkspaceId, ChannelId, ThreadId, MemberId) {
        (
            WorkspaceId(uuid::Uuid::from_u128(1)),
            ChannelId(uuid::Uuid::from_u128(2)),
            ThreadId(uuid::Uuid::from_u128(3)),
            MemberId(uuid::Uuid::from_u128(4)),
        )
    }

    /// The bridge injects only what the tool declares.
    ///
    /// It used to add all four context ids to every call and rely on the tool
    /// discarding the ones it did not want. `list_channels` takes a
    /// `workspace_id` and nothing else, so once the argument structs rejected
    /// unknown fields the `/channels` command started failing outright.
    #[test]
    fn only_declared_context_is_injected() {
        let (ws, ch, th, author) = ids();
        let args = build_mcp_arguments("list_channels", &parsed(""), ws, ch, th, author);
        let obj = args.as_object().expect("object");
        assert_eq!(obj.get("workspace_id"), Some(&json!(ws.0)));
        for undeclared in ["channel_id", "thread_id", "author_id", "text"] {
            assert!(
                !obj.contains_key(undeclared),
                "list_channels does not declare {undeclared}; injecting it is what broke /channels"
            );
        }
    }

    /// A tool that declares the thread context still gets it.
    #[test]
    fn a_thread_scoped_tool_still_receives_its_context() {
        let (ws, ch, th, author) = ids();
        let args = build_mcp_arguments("list_messages", &parsed(""), ws, ch, th, author);
        let obj = args.as_object().expect("object");
        assert_eq!(obj.get("thread_id"), Some(&json!(th.0)));
    }

    /// An unknown tool gets no injected context — fail closed, since that is
    /// exactly where guessing is least safe.
    #[test]
    fn an_unknown_tool_receives_no_injected_context() {
        let (ws, ch, th, author) = ids();
        let args = build_mcp_arguments("not_a_real_tool", &parsed(""), ws, ch, th, author);
        assert_eq!(args, json!({}), "no schema means no guessing");
    }

    /// The caller's own explicit arguments are never overwritten.
    #[test]
    fn caller_supplied_arguments_win_over_injected_context() {
        let (ws, ch, th, author) = ids();
        let mine = uuid::Uuid::from_u128(99);
        let args = build_mcp_arguments(
            "list_channels",
            &parsed(&format!(r#"{{"workspace_id":"{mine}"}}"#)),
            ws,
            ch,
            th,
            author,
        );
        assert_eq!(args["workspace_id"], json!(mine.to_string()));
    }
}
