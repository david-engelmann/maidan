//! HTTP rate limiting (Cluster 30) with optional Redis backend (Cluster 54).

mod limiter;

use std::time::Duration;

use axum::{
    body::Body,
    extract::State,
    http::{header, Request},
    middleware::Next,
    response::{IntoResponse, Response},
};

pub use limiter::{try_acquire, WindowConfig};

use crate::error::ApiError;

#[derive(Clone, Copy, Debug)]
struct RateLimitConfig {
    max: u32,
    window: Duration,
}

fn config_from_env() -> Option<RateLimitConfig> {
    let max: u32 = std::env::var("MAIDAN_RATE_LIMIT_MAX").ok()?.parse().ok()?;
    if max == 0 {
        return None;
    }
    let secs: u64 = std::env::var("MAIDAN_RATE_LIMIT_WINDOW_SECS")
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
    let Some(cfg) = config_from_env() else {
        return next.run(req).await;
    };
    if exempt_path(req.uri().path()) {
        return next.run(req).await;
    }
    let key = format!("global:{}", client_key(&req));
    let redis = state.rate_limit_redis.as_ref();
    if try_acquire(
        &key,
        WindowConfig {
            max: cfg.max,
            window: cfg.window,
        },
        redis,
    )
    .await
    {
        return next.run(req).await;
    }
    too_many(cfg.window, cfg.max)
}

pub(crate) fn too_many(window: Duration, max: u32) -> Response {
    let retry_after = window.as_secs().max(1);
    let err = ApiError::TooManyRequests(format!(
        "rate limit exceeded ({max} requests per {secs}s)",
        secs = window.as_secs()
    ));
    let mut response = err.into_response();
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
