//! HTTP rate limiting (Cluster 30) with optional Redis backend (Cluster 54).

mod limiter;

use std::time::Duration;

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

pub use limiter::{try_acquire, WindowConfig};

use crate::error::ApiError;

/// The MCP JSON-RPC POST endpoints — a rate-limit rejection here is returned as a
/// JSON-RPC error envelope (Cluster 172) so an agent's JSON-RPC layer gets a
/// typed backpressure signal instead of an opaque transport 429.
pub(crate) fn is_mcp_jsonrpc_path(path: &str) -> bool {
    path == "/mcp" || path == "/mcp/streamable"
}

#[derive(Clone, Copy, Debug)]
struct RateLimitConfig {
    max: u32,
    window: Duration,
}

fn config_from_env() -> Option<RateLimitConfig> {
    config_from_env_named("MAIDAN_RATE_LIMIT_MAX", "MAIDAN_RATE_LIMIT_WINDOW_SECS")
}

/// Per-workspace fairness limit (Cluster 110): caps total request rate for a
/// single workspace across *all* its tokens, so one tenant's heavy loop can't
/// monopolize the shared instance. Independently opt-in from the global limit.
fn workspace_config_from_env() -> Option<RateLimitConfig> {
    config_from_env_named(
        "MAIDAN_WORKSPACE_RATE_LIMIT_MAX",
        "MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS",
    )
}

fn config_from_env_named(max_var: &str, window_var: &str) -> Option<RateLimitConfig> {
    let max: u32 = std::env::var(max_var).ok()?.parse().ok()?;
    if max == 0 {
        return None;
    }
    let secs: u64 = std::env::var(window_var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
        .max(1);
    Some(RateLimitConfig {
        max,
        window: Duration::from_secs(secs),
    })
}

fn exempt_path(path: &str) -> bool {
    path.starts_with("/health") || path == "/metrics"
}

/// The workspace id segment of a `/workspaces/{wid}/…` path (or a bare
/// `/workspaces/{wid}`), for per-workspace fairness keying. `None` for
/// non-workspace-scoped paths (e.g. `/workspaces` itself, `/channels/...`).
fn workspace_id_from_path(path: &str) -> Option<&str> {
    let seg = path.strip_prefix("/workspaces/")?.split('/').next()?;
    (!seg.is_empty()).then_some(seg)
}

fn client_key(req: &Request<Body>) -> String {
    if let Some(h) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(s) = h.to_str() {
            if let Some(token) = s
                .strip_prefix("Bearer ")
                .or_else(|| s.strip_prefix("bearer "))
            {
                let n = token.len().min(40);
                return format!("bearer:{}", &token[..n]);
            }
        }
    }
    if let Some(h) = req.headers().get("x-forwarded-for") {
        if let Ok(s) = h.to_str() {
            if let Some(ip) = s.split(',').next() {
                return format!("ip:{}", ip.trim());
            }
        }
    }
    "anonymous".into()
}

pub async fn middleware(
    State(state): State<crate::state::AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let global = config_from_env();
    let workspace = workspace_config_from_env();
    if (global.is_none() && workspace.is_none()) || exempt_path(req.uri().path()) {
        return next.run(req).await;
    }
    let redis = state.rate_limit_redis.as_ref();
    let is_mcp = is_mcp_jsonrpc_path(req.uri().path());

    // Global per-client (bearer/IP) limit.
    if let Some(cfg) = global {
        let key = format!("global:{}", client_key(&req));
        if !try_acquire(&key, cfg.into(), redis).await {
            return too_many(cfg.window, cfg.max, is_mcp);
        }
    }
    // Per-workspace fairness on workspace-scoped routes (Cluster 110).
    if let Some(cfg) = workspace {
        if let Some(wid) = workspace_id_from_path(req.uri().path()) {
            let key = format!("ws:{wid}");
            if !try_acquire(&key, cfg.into(), redis).await {
                return too_many(cfg.window, cfg.max, is_mcp);
            }
        }
    }
    next.run(req).await
}

impl From<RateLimitConfig> for WindowConfig {
    fn from(c: RateLimitConfig) -> Self {
        WindowConfig {
            max: c.max,
            window: c.window,
        }
    }
}

pub(crate) fn too_many(window: Duration, max: u32, is_mcp: bool) -> Response {
    let retry_after = window.as_secs().max(1);
    let mut response = if is_mcp {
        // Structured backpressure for MCP JSON-RPC clients (Cluster 172): a
        // JSON-RPC error envelope with `retry_after_ms` in `data`, still under a
        // 429 so HTTP-level infra sees it too.
        let retry_after_ms = u64::try_from(window.as_millis()).unwrap_or(u64::MAX).max(1);
        let err = maidan_mcp::McpError::RateLimited { retry_after_ms }.to_jsonrpc();
        let body = maidan_mcp::JsonRpcResponse::failure(serde_json::Value::Null, err);
        (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response()
    } else {
        ApiError::TooManyRequests(format!(
            "rate limit exceeded ({max} requests per {secs}s)",
            secs = window.as_secs()
        ))
        .into_response()
    };
    if let Ok(v) = retry_after.to_string().parse() {
        response.headers_mut().insert(header::RETRY_AFTER, v);
    }
    response
}

/// Connect Redis when `MAIDAN_RATE_LIMIT_REDIS_URL` is set.
pub async fn connect_redis_from_env() -> Option<redis::aio::ConnectionManager> {
    let url = std::env::var("MAIDAN_RATE_LIMIT_REDIS_URL")
        .ok()?
        .trim()
        .to_string();
    if url.is_empty() {
        return None;
    }
    let client = redis::Client::open(url.as_str()).ok()?;
    let conn = redis::aio::ConnectionManager::new(client).await.ok()?;
    tracing::info!("rate limiter using Redis backend");
    Some(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_id_extracted_only_from_scoped_paths() {
        assert_eq!(
            workspace_id_from_path("/workspaces/abc/search"),
            Some("abc")
        );
        assert_eq!(workspace_id_from_path("/workspaces/abc"), Some("abc"));
        assert_eq!(
            workspace_id_from_path("/workspaces/abc/channels"),
            Some("abc")
        );
        // Not workspace-scoped → no per-workspace key.
        assert_eq!(workspace_id_from_path("/workspaces"), None);
        assert_eq!(workspace_id_from_path("/workspaces/"), None);
        assert_eq!(workspace_id_from_path("/channels/xyz/threads"), None);
        assert_eq!(workspace_id_from_path("/health"), None);
    }

    #[test]
    fn health_and_metrics_are_exempt() {
        assert!(exempt_path("/health"));
        assert!(exempt_path("/health/ready"));
        assert!(exempt_path("/metrics"));
        assert!(!exempt_path("/workspaces/abc/search"));
    }
}
