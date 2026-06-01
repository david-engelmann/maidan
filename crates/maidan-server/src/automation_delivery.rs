//! Durable signed HTTP delivery for slash commands and FSM hooks.

use maidan_types::{
    AutomationDeliveryPending, AutomationSourceKind, FsmHookId, NewAutomationDelivery,
    SlashCommandId, WorkspaceId,
};
use reqwest::Client;

use crate::fsm_hooks::resolve_fsm_secret;
use crate::slash_commands::resolve_slash_secret;
use crate::state::AppState;
use crate::webhooks::{delivery_backoff, sign_payload};

pub fn max_attempts_from_env() -> u32 {
    std::env::var("MAIDAN_AUTOMATION_MAX_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(16)
}

pub fn poll_interval_ms_from_env() -> u64 {
    std::env::var("MAIDAN_AUTOMATION_POLL_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
}

pub async fn enqueue(state: &AppState, new: NewAutomationDelivery) -> Result<i64, String> {
    state
        .store
        .enqueue_automation_delivery(new)
        .await
        .map_err(|e| e.to_string())
}

pub async fn deliver_pending(
    client: &Client,
    state: &AppState,
    delivery: &AutomationDeliveryPending,
) -> Result<(), String> {
    let secret = resolve_secret(state, delivery).await?;
    let signature = sign_payload(&secret, &delivery.payload);
    let response = client
        .post(&delivery.target_url)
        .header("Content-Type", "application/json")
        .header(&delivery.header_name, &delivery.header_value)
        .header("X-Maidan-Signature", signature)
        .header("X-Maidan-Delivery-Id", delivery.id.to_string())
        .body(delivery.payload.clone())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", response.status()))
    }
}

async fn resolve_secret(
    state: &AppState,
    delivery: &AutomationDeliveryPending,
) -> Result<String, String> {
    match delivery.source_kind {
        AutomationSourceKind::SlashCommand => {
            let reg = state
                .store
                .get_slash_command(SlashCommandId(delivery.source_id))
                .await
                .map_err(|e| e.to_string())?;
            resolve_slash_secret(&state.slash, reg.command.id, &reg.secret_ciphertext)
                .ok_or_else(|| "missing slash signing secret".to_string())
        }
        AutomationSourceKind::FsmHook => {
            let reg = state
                .store
                .get_fsm_hook(FsmHookId(delivery.source_id))
                .await
                .map_err(|e| e.to_string())?;
            resolve_fsm_secret(&state.fsm_hooks, reg.hook.id, &reg.secret_ciphertext)
                .ok_or_else(|| "missing fsm hook signing secret".to_string())
        }
    }
}

pub fn backoff(attempts: i32) -> chrono::Duration {
    delivery_backoff(attempts)
}

pub async fn enqueue_slash_http(
    state: &AppState,
    workspace_id: WorkspaceId,
    command_id: SlashCommandId,
    target_url: &str,
    command_name: &str,
    body: &str,
) -> Result<i64, String> {
    enqueue(
        state,
        NewAutomationDelivery {
            workspace_id,
            source_kind: AutomationSourceKind::SlashCommand,
            source_id: command_id.0,
            target_url: target_url.to_string(),
            header_name: "X-Maidan-Command".to_string(),
            header_value: command_name.to_string(),
            payload: body.to_string(),
        },
    )
    .await
}

pub async fn enqueue_fsm_http(
    state: &AppState,
    workspace_id: WorkspaceId,
    hook_id: FsmHookId,
    target_url: &str,
    body: &str,
) -> Result<i64, String> {
    enqueue(
        state,
        NewAutomationDelivery {
            workspace_id,
            source_kind: AutomationSourceKind::FsmHook,
            source_id: hook_id.0,
            target_url: target_url.to_string(),
            header_name: "X-Maidan-Event".to_string(),
            header_value: "thread_state_changed".to_string(),
            payload: body.to_string(),
        },
    )
    .await
}
