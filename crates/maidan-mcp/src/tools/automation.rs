//! Slash-command and FSM-hook registration/listing tool handlers.

use std::sync::Arc;

use maidan_auth::capability::{WORKSPACE_ADMIN, WORKSPACE_READ, WORKSPACE_WRITE};
use maidan_auth::{encrypt_peer_secret, AuthContext, TokenSecret};
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{content_json, required_capability};
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterSlashCommandArgs {
    workspace_id: uuid::Uuid,
    name: String,
    description: Option<String>,
    handler_kind: String,
    handler_target: String,
}

pub(super) async fn register_slash_command(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    if !auth.bypass {
        maidan_auth::require_observed_capability(
            auth,
            maidan_auth::AuthorizationSurface::Mcp,
            WORKSPACE_WRITE,
        )
        .map_err(McpError::from)?;
    }
    let a: RegisterSlashCommandArgs = crate::tools::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let name = normalize_slash_name(&a.name)?;
    let handler_kind = SlashHandlerKind::parse(&a.handler_kind).ok_or_else(|| {
        McpError::InvalidParams("handler_kind must be http, mcp_tool, or wasi".into())
    })?;
    let handler_target = match handler_kind {
        SlashHandlerKind::Http => {
            validate_http_target(&a.handler_target)?;
            a.handler_target.trim().to_string()
        }
        SlashHandlerKind::McpTool => {
            required_capability(&a.handler_target)?;
            a.handler_target.trim().to_string()
        }
        // Not just a syntax check: the sha must name an artifact this workspace
        // owns, or the command would be registrable and permanently unrunnable.
        SlashHandlerKind::Wasi => maidan_auth::resolve_wasi_handler_target(
            store.as_ref(),
            auth,
            workspace_id,
            &a.handler_target,
        )
        .await
        .map_err(|e| match e {
            maidan_auth::WasiTargetError::Invalid(msg) => McpError::InvalidParams(msg),
            // A store that could not answer is not invalid params.
            maidan_auth::WasiTargetError::Store(err) => McpError::from(err),
        })?,
    };
    let secret_ciphertext = if handler_kind == SlashHandlerKind::Http {
        let key = maidan_auth::encryption_key_from_env().map_err(|_| {
            McpError::InvalidParams(
                "FEDERATION_ENCRYPTION_KEY must be set for http slash handlers".into(),
            )
        })?;
        let secret = TokenSecret::generate();
        let ciphertext = encrypt_peer_secret(secret.as_str(), &key)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        let command = store
            .create_slash_command(NewSlashCommand {
                workspace_id,
                name,
                description: a.description,
                handler_kind,
                handler_target,
                secret_ciphertext: ciphertext,
                created_by_token: auth.token_id,
            })
            .await
            .map_err(McpError::from)?;
        record_slash_registration(store, auth, workspace_id, &command).await;
        return Ok(content_json(&json!({
            "command": command,
            "secret": secret.as_str()
        })));
    } else {
        String::new()
    };
    let command = store
        .create_slash_command(NewSlashCommand {
            workspace_id,
            name,
            description: a.description,
            handler_kind,
            handler_target,
            secret_ciphertext,
            created_by_token: auth.token_id,
        })
        .await
        .map_err(McpError::from)?;
    record_slash_registration(store, auth, workspace_id, &command).await;
    Ok(content_json(&command))
}

/// The audit row naming who registered a command, as the REST route writes
/// it: the command runs only while that token could register it again.
async fn record_slash_registration(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    command: &SlashCommand,
) {
    let event = NewAuditEvent {
        scope: AuditScope::Workspace(workspace_id),
        actor_id: (!auth.bypass).then_some(auth.actor_id),
        action: "slash_command.create".into(),
        target_kind: Some("slash_command".into()),
        target_id: Some(command.id.0),
        metadata: json!({
            "workspace_id": workspace_id.0,
            "created_by_token": auth.token_id.map(|token| token.0),
            "member_id": (!auth.bypass).then_some(auth.member_id.0),
        }),
    };
    if let Err(err) = store.append_audit(event).await {
        tracing::error!(target: "audit", %err, action = "slash_command.create", "audit.write_failed");
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListSlashCommandsArgs {
    workspace_id: uuid::Uuid,
}

pub(super) async fn list_slash_commands(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    if !auth.bypass {
        maidan_auth::require_observed_capability(
            auth,
            maidan_auth::AuthorizationSurface::Mcp,
            WORKSPACE_READ,
        )
        .map_err(McpError::from)?;
    }
    let a: ListSlashCommandsArgs = crate::tools::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let commands = store
        .list_slash_commands(workspace_id)
        .await
        .map_err(McpError::from)?;
    Ok(content_json(&commands))
}

fn normalize_slash_name(name: &str) -> Result<String, McpError> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    if normalized.is_empty() || normalized.len() > 32 {
        return Err(McpError::InvalidParams(
            "slash command name must be 1-32 characters".into(),
        ));
    }
    if !normalized
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(McpError::InvalidParams(
            "slash command name may only contain a-z, 0-9, _, -".into(),
        ));
    }
    Ok(normalized)
}

fn validate_http_target(url: &str) -> Result<(), McpError> {
    maidan_auth::validate_egress_target(url)
        .map(|_| ())
        .map_err(|error| McpError::InvalidParams(error.to_string()))
}

fn parse_opt_state_mcp(raw: Option<String>) -> Result<Option<ThreadState>, McpError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                ThreadState::parse(trimmed)
                    .ok_or_else(|| {
                        McpError::InvalidParams(format!("unknown thread state: {trimmed}"))
                    })
                    .map(Some)
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterFsmHookArgs {
    workspace_id: uuid::Uuid,
    label: Option<String>,
    from_state: Option<String>,
    to_state: Option<String>,
    handler_kind: String,
    handler_target: String,
}

pub(super) async fn register_fsm_hook(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    if !auth.bypass {
        maidan_auth::require_observed_capability(
            auth,
            maidan_auth::AuthorizationSurface::Mcp,
            WORKSPACE_ADMIN,
        )
        .map_err(McpError::from)?;
    }
    let a: RegisterFsmHookArgs = crate::tools::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let from_state = parse_opt_state_mcp(a.from_state)?;
    let to_state = parse_opt_state_mcp(a.to_state)?;
    let handler_kind = SlashHandlerKind::parse(&a.handler_kind).ok_or_else(|| {
        McpError::InvalidParams("handler_kind must be http, mcp_tool, or wasi".into())
    })?;
    match handler_kind {
        SlashHandlerKind::Http => validate_http_target(&a.handler_target)?,
        SlashHandlerKind::McpTool => {
            // The hook runs the tool as this credential, so a tool it may not
            // call would only ever be refused when the hook fires.
            let required = required_capability(&a.handler_target)?;
            if !auth.bypass {
                maidan_auth::require_observed_capability(
                    auth,
                    maidan_auth::AuthorizationSurface::Mcp,
                    required,
                )
                .map_err(McpError::from)?;
            }
        }
        SlashHandlerKind::Wasi => {
            return Err(McpError::InvalidParams(
                "wasi handlers are not supported for fsm hooks".into(),
            ));
        }
    }
    let secret_ciphertext = if handler_kind == SlashHandlerKind::Http {
        let key = maidan_auth::encryption_key_from_env().map_err(|_| {
            McpError::InvalidParams(
                "FEDERATION_ENCRYPTION_KEY must be set for http fsm hooks".into(),
            )
        })?;
        let secret = TokenSecret::generate();
        let ciphertext = encrypt_peer_secret(secret.as_str(), &key)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        let hook = store
            .create_fsm_hook(NewFsmHook {
                workspace_id,
                label: a.label,
                from_state,
                to_state,
                handler_kind,
                handler_target: a.handler_target.trim().to_string(),
                secret_ciphertext: ciphertext,
                created_by_token: auth.token_id,
            })
            .await
            .map_err(McpError::from)?;
        return Ok(content_json(&json!({
            "hook": hook,
            "secret": secret.as_str()
        })));
    } else {
        String::new()
    };
    let hook = store
        .create_fsm_hook(NewFsmHook {
            workspace_id,
            label: a.label,
            from_state,
            to_state,
            handler_kind,
            handler_target: a.handler_target.trim().to_string(),
            secret_ciphertext,
            created_by_token: auth.token_id,
        })
        .await
        .map_err(McpError::from)?;
    Ok(content_json(&hook))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListFsmHooksArgs {
    workspace_id: uuid::Uuid,
}

pub(super) async fn list_fsm_hooks(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    if !auth.bypass {
        maidan_auth::require_observed_capability(
            auth,
            maidan_auth::AuthorizationSurface::Mcp,
            WORKSPACE_READ,
        )
        .map_err(McpError::from)?;
    }
    let a: ListFsmHooksArgs = crate::tools::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let hooks = store
        .list_fsm_hooks(workspace_id)
        .await
        .map_err(McpError::from)?;
    Ok(content_json(&hooks))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeSlashArgs {
    workspace_id: uuid::Uuid,
    command_id: uuid::Uuid,
}

/// Revoke a slash command. Twin of `DELETE /workspaces/{wid}/slash-commands/{cid}`:
/// `revoke_slash_command`, then drop the in-memory signing secret when the row
/// belongs to this workspace.
pub(super) async fn revoke_slash_command(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RevokeSlashArgs = crate::tools::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let command_id = SlashCommandId(a.command_id);
    let command = server
        .store
        .revoke_slash_command(workspace_id, command_id)
        .await?;
    server.forget_slash_secret(a.command_id);
    Ok(content_json(&command))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeFsmArgs {
    workspace_id: uuid::Uuid,
    hook_id: uuid::Uuid,
}

/// Revoke an FSM hook. Twin of `DELETE /workspaces/{wid}/fsm-hooks/{hid}`.
pub(super) async fn revoke_fsm_hook(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RevokeFsmArgs = crate::tools::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let hook_id = FsmHookId(a.hook_id);
    let hook = server.store.revoke_fsm_hook(workspace_id, hook_id).await?;
    server.forget_fsm_secret(a.hook_id);
    Ok(content_json(&hook))
}
