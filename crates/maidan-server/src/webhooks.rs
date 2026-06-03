//! Outbound webhook admin routes and HMAC-signed delivery helpers.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    decrypt_peer_secret, encrypt_peer_secret, AuthContext, TokenSecret,
};
use maidan_types::{
    Event, EventKind, NewWebhookSubscription, WebhookSubscription, WebhookSubscriptionId,
    WorkspaceId,
};
use reqwest::Client;
use serde::Serialize;
use sha2::Sha256;
use utoipa::ToSchema;

use crate::dto::{
    CreateWebhook, MentionWebhookConfig, MintWebhookResponse, SetMentionWebhook, WebhookResponse,
};
use crate::error::{ApiError, ApiJson};
use crate::state::{AppState, WebhookRuntime};

type ApiResult<T> = Result<T, ApiError>;
type HmacSha256 = Hmac<Sha256>;

fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

pub fn remember_webhook_secret(
    secrets: &Arc<RwLock<HashMap<WebhookSubscriptionId, String>>>,
    id: WebhookSubscriptionId,
    secret: String,
) {
    if let Ok(mut guard) = secrets.write() {
        guard.insert(id, secret);
    }
}

pub fn forget_webhook_secret(
    secrets: &Arc<RwLock<HashMap<WebhookSubscriptionId, String>>>,
    id: WebhookSubscriptionId,
) {
    if let Ok(mut guard) = secrets.write() {
        guard.remove(&id);
    }
}

pub async fn hydrate_webhook_secrets(state: &AppState) -> Result<(), String> {
    let Some(key) = state.webhooks.encryption_key.as_deref() else {
        return Ok(());
    };
    let subs = state
        .store
        .list_enabled_webhook_subscriptions()
        .await
        .map_err(|e| e.to_string())?;
    for sub in subs {
        match decrypt_peer_secret(&sub.secret_ciphertext, key) {
            Ok(secret) => {
                remember_webhook_secret(&state.webhooks.secrets, sub.subscription.id, secret)
            }
            Err(err) => tracing::warn!(
                webhook = %sub.subscription.id,
                error = %err,
                "failed to decrypt stored webhook secret"
            ),
        }
    }
    Ok(())
}

pub fn resolve_webhook_secret(
    runtime: &WebhookRuntime,
    subscription_id: WebhookSubscriptionId,
    secret_ciphertext: &str,
) -> Option<String> {
    if let Ok(guard) = runtime.secrets.read() {
        if let Some(secret) = guard.get(&subscription_id) {
            return Some(secret.clone());
        }
    }
    let key = runtime.encryption_key.as_deref()?;
    let secret = decrypt_peer_secret(secret_ciphertext, key).ok()?;
    remember_webhook_secret(&runtime.secrets, subscription_id, secret.clone());
    Some(secret)
}

fn validate_webhook_url(url: &str) -> ApiResult<()> {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(ApiError::BadRequest(
            "webhook url must use http or https".into(),
        ));
    }
    if trimmed.len() > 2048 {
        return Err(ApiError::BadRequest("webhook url too long".into()));
    }
    if trimmed.as_bytes().contains(&b' ') {
        return Err(ApiError::BadRequest("invalid webhook url".into()));
    }
    Ok(())
}

fn parse_event_kinds(kinds: &[String]) -> ApiResult<Vec<String>> {
    if kinds.is_empty() {
        return Err(ApiError::BadRequest(
            "event_kinds must include at least one EventKind".into(),
        ));
    }
    let mut out = Vec::with_capacity(kinds.len());
    for k in kinds {
        let parsed = EventKind::parse(k)
            .ok_or_else(|| ApiError::BadRequest(format!("unknown event kind: {k}")))?;
        let canonical = parsed.as_str().to_string();
        if !out.contains(&canonical) {
            out.push(canonical);
        }
    }
    Ok(out)
}

pub async fn create_webhook(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateWebhook>,
) -> ApiResult<(StatusCode, Json<MintWebhookResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    validate_webhook_url(&body.url)?;
    let event_kinds = parse_event_kinds(&body.event_kinds)?;

    let secret = TokenSecret::generate();
    let key = state.webhooks.encryption_key.as_deref().ok_or_else(|| {
        ApiError::Internal("FEDERATION_ENCRYPTION_KEY must be set to create webhooks".into())
    })?;
    let secret_ciphertext =
        encrypt_peer_secret(secret.as_str(), key).map_err(|e| ApiError::Internal(e.to_string()))?;

    let subscription = state
        .store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id,
            url: body.url.trim().to_string(),
            label: body.label,
            event_kinds,
            secret_ciphertext,
        })
        .await?;

    remember_webhook_secret(
        &state.webhooks.secrets,
        subscription.id,
        secret.as_str().to_string(),
    );

    Ok((
        StatusCode::CREATED,
        Json(MintWebhookResponse {
            webhook: WebhookResponse::from(subscription),
            secret: secret.as_str().to_string(),
        }),
    ))
}

pub async fn list_webhooks(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<WebhookResponse>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let subs = state.store.list_webhook_subscriptions(workspace_id).await?;
    Ok(Json(subs.into_iter().map(WebhookResponse::from).collect()))
}

pub async fn get_mention_webhook(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<MentionWebhookConfig>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let webhook_id = state
        .store
        .get_workspace_mention_webhook_id(workspace_id)
        .await?;
    Ok(Json(MentionWebhookConfig { webhook_id }))
}

pub async fn set_mention_webhook(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetMentionWebhook>,
) -> ApiResult<Json<MentionWebhookConfig>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let webhook_id = body.webhook_id.map(WebhookSubscriptionId);
    if let Some(id) = webhook_id {
        let sub = state.store.get_webhook_subscription(id).await?;
        if sub.subscription.workspace_id != workspace_id || sub.subscription.revoked_at.is_some() {
            return Err(ApiError::BadRequest(
                "webhook must be an active subscription in this workspace".into(),
            ));
        }
    }
    state
        .store
        .set_workspace_mention_webhook_id(workspace_id, webhook_id)
        .await?;
    Ok(Json(MentionWebhookConfig { webhook_id }))
}

pub async fn revoke_webhook(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, webhook_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(workspace_id);
    let webhook_id = WebhookSubscriptionId(webhook_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let sub = state.store.revoke_webhook_subscription(webhook_id).await?;
    if sub.workspace_id != workspace_id {
        return Err(ApiError::NotFound);
    }
    forget_webhook_secret(&state.webhooks.secrets, webhook_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
pub struct WebhookPayload<'a> {
    pub log_id: i64,
    pub kind: &'static str,
    pub occurred_at: DateTime<Utc>,
    pub event: &'a Event,
}

pub fn build_payload(log_id: i64, event: &Event) -> Result<String, serde_json::Error> {
    let payload = WebhookPayload {
        log_id,
        kind: event.kind().as_str(),
        occurred_at: Utc::now(),
        event,
    };
    serde_json::to_string(&payload)
}

pub fn sign_payload(secret: &str, body: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

pub fn verify_signature(secret: &str, body: &str, header: &str) -> bool {
    let expected = sign_payload(secret, body);
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), header.as_bytes()).into()
}

pub async fn deliver_http(
    client: &Client,
    url: &str,
    delivery_id: i64,
    kind: EventKind,
    secret: &str,
    body: &str,
) -> Result<(), String> {
    let signature = sign_payload(secret, body);
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("X-Maidan-Signature", signature)
        .header("X-Maidan-Event", kind.as_str())
        .header("X-Maidan-Delivery-Id", delivery_id.to_string())
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", response.status()))
    }
}

pub fn event_kind_from_payload(payload: &str) -> EventKind {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
        .and_then(|s| EventKind::parse(&s))
        .unwrap_or(EventKind::MessagePosted)
}

pub fn kinds_match(subscription: &WebhookSubscription, kind: &EventKind) -> bool {
    let needle = kind.clone().as_str();
    subscription.event_kinds.iter().any(|k| k == needle)
}

pub fn delivery_backoff(attempts: i32) -> chrono::Duration {
    let secs = 2_i64.saturating_pow(attempts.min(8) as u32).min(3600);
    chrono::Duration::seconds(secs)
}

pub fn max_attempts_from_env() -> u32 {
    std::env::var("MAIDAN_WEBHOOK_MAX_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(16)
}

pub fn poll_interval_ms_from_env() -> u64 {
    std::env::var("MAIDAN_WEBHOOK_POLL_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookDeliverySummary {
    pub delivered: bool,
}
