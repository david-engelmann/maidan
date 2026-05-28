//! Optional global HTTP rate limiting (Cluster 30).
//!
//! Enabled when `MAIDAN_RATE_LIMIT_MAX` is set to a positive integer.
//! Keys requests by bearer token prefix, else first `X-Forwarded-For` hop,
//! else `anonymous`. `/health/*` and `/metrics` are exempt.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    http::{header, Request},
    middleware::Next,
    response::{IntoResponse, Response},
};

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

struct WindowCounter {
    window_start: Instant,
    count: u32,
}

static BUCKETS: LazyLock<Mutex<HashMap<String, WindowCounter>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn check_and_increment(key: &str, cfg: RateLimitConfig) -> bool {
    let now = Instant::now();
    let mut buckets = BUCKETS.lock().expect("rate limit buckets");
    let entry = buckets.entry(key.to_string()).or_insert(WindowCounter {
        window_start: now,
        count: 0,
    });
    if now.duration_since(entry.window_start) >= cfg.window {
        entry.window_start = now;
        entry.count = 0;
    }
    if entry.count >= cfg.max {
        return false;
    }
    entry.count += 1;
    true
}

pub async fn middleware(req: Request<Body>, next: Next) -> Response {
    let Some(cfg) = config_from_env() else {
        return next.run(req).await;
    };
    if exempt_path(req.uri().path()) {
        return next.run(req).await;
    }
    let key = client_key(&req);
    if check_and_increment(&key, cfg) {
        return next.run(req).await;
    }
    let retry_after = cfg.window.as_secs().max(1);
    let err = ApiError::TooManyRequests(format!(
        "rate limit exceeded ({max} requests per {secs}s)",
        max = cfg.max,
        secs = cfg.window.as_secs()
    ));
    let mut response = err.into_response();
    if let Ok(v) = retry_after.to_string().parse() {
        response.headers_mut().insert(header::RETRY_AFTER, v);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_resets_after_window() {
        let cfg = RateLimitConfig {
            max: 2,
            window: Duration::from_millis(50),
        };
        let key = "test-key";
        assert!(check_and_increment(key, cfg));
        assert!(check_and_increment(key, cfg));
        assert!(!check_and_increment(key, cfg));
        std::thread::sleep(Duration::from_millis(60));
        assert!(check_and_increment(key, cfg));
    }
}
