//! Per-token capability quotas (Cluster 54).

use std::time::Duration;

use axum::{body::Body, extract::State, http::Request, middleware::Next, response::Response};
use maidan_auth::{
    capability::{
        ARTIFACT_UPLOAD, MESSAGE_POST, SEARCH_QUERY, THREAD_TRANSITION, WORKSPACE_READ,
        WORKSPACE_WRITE,
    },
    AuthContext,
};
use maidan_types::TokenQuota;

use maidan_types::ApiTokenId;

use crate::error::ApiError;
use crate::rate_limit::{too_many, try_acquire, WindowConfig};
use crate::state::AppState;

fn route_capability(method: &str, path: &str) -> Option<&'static str> {
    if path.contains("/messages") && method == "POST" {
        return Some(MESSAGE_POST);
    }
    if path.starts_with("/threads/") && method == "POST" {
        return Some(THREAD_TRANSITION);
    }
    if path.contains("/search") {
        return Some(SEARCH_QUERY);
    }
    if path.contains("/artifacts") && matches!(method, "POST" | "PUT") {
        return Some(ARTIFACT_UPLOAD);
    }
    if path.contains("/purge")
        || (path.starts_with("/workspaces/") && method == "DELETE")
        || matches!(method, "POST" | "PUT" | "PATCH" | "DELETE")
    {
        return Some(WORKSPACE_WRITE);
    }
    if matches!(method, "GET" | "HEAD") {
        return Some(WORKSPACE_READ);
    }
    None
}

pub fn validate_token_quotas(quotas: &[TokenQuota], token_caps: &[String]) -> Result<(), ApiError> {
    for q in quotas {
        if q.max_per_window == 0 || q.window_secs == 0 {
            return Err(ApiError::BadRequest(
                "quota max_per_window and window_secs must be positive".into(),
            ));
        }
        if !maidan_auth::capability::is_known(&q.capability) {
            return Err(ApiError::BadRequest(format!(
                "unknown capability in quota: {}",
                q.capability
            )));
        }
        if !token_caps.iter().any(|c| c == &q.capability) {
            return Err(ApiError::BadRequest(format!(
                "quota capability {} not granted on token",
                q.capability
            )));
        }
    }
    Ok(())
}

/// Enforce per-token quota for a capability (HTTP routes and MCP `tools/call`, Cluster 64).
pub async fn enforce_token_quota(
    state: &AppState,
    token_id: ApiTokenId,
    cap: &str,
) -> Result<(), ApiError> {
    let quotas = state.store.list_token_quotas(token_id).await?;
    let Some(q) = quotas.iter().find(|q| q.capability == cap) else {
        return Ok(());
    };
    let key = format!("quota:{}:{cap}", token_id.0);
    let cfg = WindowConfig {
        max: q.max_per_window,
        window: Duration::from_secs(q.window_secs),
    };
    if !try_acquire(&key, cfg, state.rate_limit_redis.as_ref()).await {
        return Err(ApiError::TooManyRequests(format!(
            "capability quota exceeded for {cap} ({max} per {secs}s)",
            max = cfg.max,
            secs = cfg.window.as_secs()
        )));
    }
    Ok(())
}

pub async fn middleware(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let method = req.method().as_str();
    let path = req.uri().path();
    if let Some(auth) = req.extensions().get::<AuthContext>() {
        if !auth.bypass {
            if let Some(token_id) = auth.token_id {
                if let Some(cap) = route_capability(method, path) {
                    match state.store.list_token_quotas(token_id).await {
                        Ok(quotas) => {
                            if let Some(q) = quotas.iter().find(|q| q.capability == cap) {
                                let key = format!("quota:{}:{cap}", token_id.0);
                                let cfg = WindowConfig {
                                    max: q.max_per_window,
                                    window: Duration::from_secs(q.window_secs),
                                };
                                if !try_acquire(&key, cfg, state.rate_limit_redis.as_ref()).await {
                                    return too_many(cfg.window, cfg.max);
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "token quota list failed");
                        }
                    }
                }
            }
        }
    }
    next.run(req).await
}
