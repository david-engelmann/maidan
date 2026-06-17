//! Slash-command and FSM-hook registration/listing tool handlers.

use std::sync::Arc;

use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};
use maidan_auth::{encrypt_peer_secret, AuthContext, TokenSecret};
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{content_json, required_capability};
use crate::error::McpError;

#[derive(Deserialize)]
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
        auth.require_capability(WORKSPACE_WRITE)
            .map_err(McpError::from)?;
    }
    let a: RegisterSlashCommandArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let name = normalize_slash_name(&a.name)?;
    let handler_kind = SlashHandlerKind::parse(&a.handler_kind)
        .ok_or_else(|| McpError::InvalidParams("handler_kind must be http or mcp_tool".into()))?;
    match handler_kind {
        SlashHandlerKind::Http => validate_http_target(&a.handler_target)?,
        SlashHandlerKind::McpTool => {
            required_capability(&a.handler_target)?;
        }
    }
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
                handler_target: a.handler_target.trim().to_string(),
                secret_ciphertext: ciphertext,
            })
            .await
            .map_err(McpError::from)?;
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
            handler_target: a.handler_target.trim().to_string(),
            secret_ciphertext,
        })
        .await
        .map_err(McpError::from)?;
    Ok(content_json(&command))
}

#[derive(Deserialize)]
struct ListSlashCommandsArgs {
    workspace_id: uuid::Uuid,
}

pub(super) async fn list_slash_commands(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_READ)
            .map_err(McpError::from)?;
    }
    let a: ListSlashCommandsArgs = serde_json::from_value(args.clone())?;
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
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(McpError::InvalidParams(
            "handler_target url must use http or https".into(),
        ));
    }
    if trimmed.len() > 2048 || trimmed.as_bytes().contains(&b' ') {
        return Err(McpError::InvalidParams("invalid handler url".into()));
    }
    Ok(())
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
        auth.require_capability(WORKSPACE_WRITE)
            .map_err(McpError::from)?;
    }
    let a: RegisterFsmHookArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let from_state = parse_opt_state_mcp(a.from_state)?;
    let to_state = parse_opt_state_mcp(a.to_state)?;
    let handler_kind = SlashHandlerKind::parse(&a.handler_kind)
        .ok_or_else(|| McpError::InvalidParams("handler_kind must be http or mcp_tool".into()))?;
    match handler_kind {
        SlashHandlerKind::Http => validate_http_target(&a.handler_target)?,
        SlashHandlerKind::McpTool => {
            required_capability(&a.handler_target)?;
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
        })
        .await
        .map_err(McpError::from)?;
    Ok(content_json(&hook))
}

#[derive(Deserialize)]
struct ListFsmHooksArgs {
    workspace_id: uuid::Uuid,
}

pub(super) async fn list_fsm_hooks(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_READ)
            .map_err(McpError::from)?;
    }
    let a: ListFsmHooksArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let hooks = store
        .list_fsm_hooks(workspace_id)
        .await
        .map_err(McpError::from)?;
    Ok(content_json(&hooks))
}
