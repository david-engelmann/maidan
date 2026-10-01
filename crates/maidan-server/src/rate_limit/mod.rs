//! HTTP rate limiting with optional Redis backend.

mod limiter;

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{header, Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

pub use limiter::{try_acquire, WindowConfig};

use crate::error::ApiError;

/// The MCP JSON-RPC POST endpoints — a rate-limit rejection here is returned as
/// a JSON-RPC error envelope so an agent's JSON-RPC layer gets a typed
/// backpressure signal instead of an opaque transport 429.
pub(crate) fn is_mcp_jsonrpc_path(path: &str) -> bool {
    path == "/mcp" || path == "/mcp/streamable"
}

#[derive(Clone, Copy, Debug)]
struct RateLimitConfig {
    max: u32,
    window: Duration,
}

/// Built-in global per-client limit used when `MAIDAN_RATE_LIMIT_MAX` is unset
/// and the bootstrap enabled the default: 1200 requests / 60 s per bearer/IP —
/// ~20 req/s sustained, generous for a real agent but a firm floor against a
/// runaway or abusive client on a deployment that configured nothing.
const DEFAULT_GLOBAL_MAX: u32 = 1200;
const DEFAULT_GLOBAL_WINDOW_SECS: u64 = 60;

/// Built-in per-workspace limit, on the same terms as the global one: 6000
/// requests / 60 s for one workspace across all its tokens, ~100 req/s. That is
/// five clients each running at the per-client ceiling above, so a busy
/// workspace meets its own clients' limits long before its shared one. It is
/// also well under what one node serves (666–1586 req/s on the SQLite
/// benchmark, `docs/Benchmark.md`), so one tenant cannot take the instance.
const DEFAULT_WORKSPACE_MAX: u32 = 6000;
const DEFAULT_WORKSPACE_WINDOW_SECS: u64 = 60;
// A workspace's shared limit must sit well above one client's, or a single
// busy agent would meet the workspace cap before its own.
const _: () = assert!(
    DEFAULT_WORKSPACE_WINDOW_SECS == DEFAULT_GLOBAL_WINDOW_SECS
        && DEFAULT_WORKSPACE_MAX >= 5 * DEFAULT_GLOBAL_MAX
);

/// One limit's environment names and its built-in default.
struct LimitSpec {
    max_var: &'static str,
    window_var: &'static str,
    default: RateLimitConfig,
}

const GLOBAL: LimitSpec = LimitSpec {
    max_var: "MAIDAN_RATE_LIMIT_MAX",
    window_var: "MAIDAN_RATE_LIMIT_WINDOW_SECS",
    default: RateLimitConfig {
        max: DEFAULT_GLOBAL_MAX,
        window: Duration::from_secs(DEFAULT_GLOBAL_WINDOW_SECS),
    },
};

/// Per-workspace fairness limit: caps total request rate for a single workspace
/// across *all* its tokens, so one tenant's heavy loop can't monopolize the
/// shared instance. Resolved independently of the global limit.
const WORKSPACE: LimitSpec = LimitSpec {
    max_var: "MAIDAN_WORKSPACE_RATE_LIMIT_MAX",
    window_var: "MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS",
    default: RateLimitConfig {
        max: DEFAULT_WORKSPACE_MAX,
        window: Duration::from_secs(DEFAULT_WORKSPACE_WINDOW_SECS),
    },
};

fn resolve_global(default_on: bool) -> Option<RateLimitConfig> {
    resolve_env(&GLOBAL, default_on)
}

fn resolve_workspace(default_on: bool) -> Option<RateLimitConfig> {
    resolve_env(&WORKSPACE, default_on)
}

fn resolve_env(spec: &LimitSpec, default_on: bool) -> Option<RateLimitConfig> {
    resolve(
        spec,
        std::env::var(spec.max_var).ok().as_deref(),
        std::env::var(spec.window_var).ok().as_deref(),
        default_on,
    )
}

/// Resolve a limit: an explicit max always wins (including `0`/invalid →
/// disabled); otherwise apply the built-in default when `default_on` (the
/// server bootstrap sets it; tests leave it off).
fn resolve(
    spec: &LimitSpec,
    max: Option<&str>,
    window: Option<&str>,
    default_on: bool,
) -> Option<RateLimitConfig> {
    match max {
        Some(max) => config_from(max, window),
        None => default_on.then_some(spec.default),
    }
}

fn config_from(max: &str, window: Option<&str>) -> Option<RateLimitConfig> {
    let max: u32 = max.parse().ok()?;
    if max == 0 {
        return None;
    }
    let secs: u64 = window.and_then(|s| s.parse().ok()).unwrap_or(60).max(1);
    Some(RateLimitConfig {
        max,
        window: Duration::from_secs(secs),
    })
}

pub(crate) fn exempt_path(path: &str) -> bool {
    path.starts_with("/health") || path == "/metrics"
}

/// The workspace id segment of a `/workspaces/{wid}/…` path (or a bare
/// `/workspaces/{wid}`), for per-workspace fairness keying. `None` for
/// non-workspace-scoped paths (e.g. `/workspaces` itself, `/channels/...`).
fn workspace_id_from_path(path: &str) -> Option<&str> {
    let seg = path.strip_prefix("/workspaces/")?.split('/').next()?;
    (!seg.is_empty()).then_some(seg)
}

fn trusted_proxy_hops() -> usize {
    std::env::var("MAIDAN_TRUSTED_PROXY_HOPS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(0)
}

fn forwarded_client_ip(header: &str, peer: IpAddr, trusted_hops: usize) -> Option<IpAddr> {
    if trusted_hops == 0 {
        return Some(peer);
    }
    let mut chain = header
        .split(',')
        .map(str::trim)
        .map(str::parse::<IpAddr>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    chain.push(peer);
    chain
        .len()
        .checked_sub(trusted_hops + 1)
        .and_then(|index| chain.get(index).copied())
}

fn raw_bearer(req: &Request<Body>) -> Option<&str> {
    let header_value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    header_value
        .strip_prefix("Bearer ")
        .or_else(|| header_value.strip_prefix("bearer "))
}

/// `Bearer` as [`crate::auth::parse_bearer`] accepts it. The outer limiter
/// defers only a bearer that middleware will actually resolve; a lowercase
/// scheme is not one of those.
fn presented_bearer(req: &Request<Body>) -> Option<&str> {
    let header_value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = header_value.strip_prefix("Bearer ")?.trim();
    (!token.is_empty()).then_some(token)
}

fn bearer_client_key(token: &str) -> String {
    let n = token.len().min(40);
    format!("bearer:{}", &token[..n])
}

fn ip_client_key(req: &Request<Body>) -> String {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    if let Some(peer) = peer {
        let ip = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| forwarded_client_ip(value, peer, trusted_proxy_hops()))
            .unwrap_or(peer);
        return format!("ip:{ip}");
    }
    "anonymous".into()
}

fn client_key(req: &Request<Body>) -> String {
    if let Some(token) = raw_bearer(req) {
        return bearer_client_key(token);
    }
    ip_client_key(req)
}

fn at_or_under(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{root}/"))
}

/// `POST /workspaces/{wid}/members` is the ungated bootstrap seed, not the
/// bearer-auth router. Charging it as a verified bearer would skip the limit
/// whenever bootstrap is what answers.
fn bootstrap_member_create(path: &str) -> bool {
    let mut parts = path.split('/');
    parts.next() == Some("")
        && parts.next() == Some("workspaces")
        && parts.next().is_some_and(|segment| !segment.is_empty())
        && parts.next() == Some("members")
        && parts.next().is_none()
}

/// Routes whose bearer is resolved by `auth::middleware`, the UI session
/// middleware, or peer ingest. Anywhere else a bearer is not a credential
/// this limiter has verified, so it does not name a bucket.
fn bearer_resolved_later(method: &Method, path: &str) -> bool {
    if method == Method::POST && bootstrap_member_create(path) {
        return false;
    }
    const ROOTS: &[&str] = &[
        "/mcp",
        "/agui",
        "/members",
        "/threads",
        "/dm",
        "/group-dms",
        "/channels",
        "/messages",
        "/artifacts",
        "/tokens",
        "/operator",
        "/scim",
        "/approval-gates",
        "/task-schedules",
        "/me",
        "/references",
        "/capability-sets",
    ];
    if ROOTS.iter().any(|root| at_or_under(path, root)) {
        return true;
    }
    // The collection itself is the bootstrap create, which does not resolve
    // a bearer. Nested workspace routes do.
    if path.starts_with("/workspaces/") {
        return true;
    }
    path.starts_with("/a2a/v1/rpc")
        || path.starts_with("/a2a/v1/message")
        || path.starts_with("/a2a/v1/tasks")
        || path == "/a2a/v1/extendedAgentCard"
        || path == "/a2a/v1/events"
        || path.starts_with("/ui/api/")
        || path.starts_with("/auth/session/")
}

fn defer_global_bearer(state: &crate::state::AppState, req: &Request<Body>) -> bool {
    !state.auth_disabled
        && presented_bearer(req).is_some()
        && bearer_resolved_later(req.method(), req.uri().path())
}

/// A caller authenticated into the workspace named by the path. A bypass
/// caller is authenticated into every workspace, matching `ensure_workspace`.
/// Anyone else must present that workspace's own credential: knowing the id
/// is not enough, and neither is a token for a different workspace.
pub(crate) fn authenticated_into_path(
    bypass: bool,
    workspace_id: &maidan_types::WorkspaceId,
    path: &str,
) -> bool {
    let Some(wid) = workspace_id_from_path(path) else {
        return false;
    };
    bypass || workspace_id.to_string() == wid
}

async fn enforce_global(
    state: &crate::state::AppState,
    client: &str,
    is_mcp: bool,
) -> Result<(), Response> {
    let Some(cfg) = resolve_global(state.rate_limit_default_on) else {
        return Ok(());
    };
    let key = format!("global:{client}");
    if try_acquire(&key, cfg.into(), state.rate_limit_redis.as_ref()).await {
        Ok(())
    } else {
        Err(too_many(cfg.window, cfg.max, is_mcp))
    }
}

/// The per-client bucket for a bearer that resolved. An invented bearer must
/// not reach this: it would open a fresh bucket per secret.
pub(crate) async fn enforce_verified_bearer(
    state: &crate::state::AppState,
    secret: &str,
    is_mcp: bool,
) -> Result<(), Response> {
    enforce_global(state, &bearer_client_key(secret), is_mcp).await
}

/// The per-client bucket for a bearer that did not resolve, shared with
/// clients that presented no bearer: the socket IP (or the forwarded client
/// when proxy hops are declared).
/// Owned client-IP key, taken before an `.await` so the request itself is
/// not borrowed across one (that future would not be `Send`).
pub(crate) fn client_ip_key(req: &Request<Body>) -> String {
    ip_client_key(req)
}

pub(crate) async fn enforce_client_key(
    state: &crate::state::AppState,
    client: &str,
    is_mcp: bool,
) -> Result<(), Response> {
    enforce_global(state, client, is_mcp).await
}

/// Charge `ws:{wid}` only when this caller is authenticated into that
/// workspace. A miss spends nothing of the target's budget.
pub(crate) async fn enforce_workspace(
    state: &crate::state::AppState,
    path: &str,
    bypass: bool,
    workspace_id: &maidan_types::WorkspaceId,
) -> Result<(), Response> {
    if !authenticated_into_path(bypass, workspace_id, path) {
        return Ok(());
    }
    let Some(cfg) = resolve_workspace(state.rate_limit_default_on) else {
        return Ok(());
    };
    let Some(wid) = workspace_id_from_path(path) else {
        return Ok(());
    };
    let key = format!("ws:{wid}");
    if try_acquire(&key, cfg.into(), state.rate_limit_redis.as_ref()).await {
        Ok(())
    } else {
        Err(too_many(cfg.window, cfg.max, is_mcp_jsonrpc_path(path)))
    }
}

pub async fn middleware(
    State(state): State<crate::state::AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let global = resolve_global(state.rate_limit_default_on);
    let workspace_on = resolve_workspace(state.rate_limit_default_on).is_some();
    if (global.is_none() && !workspace_on) || exempt_path(req.uri().path()) {
        return next.run(req).await;
    }
    let redis = state.rate_limit_redis.clone();
    let is_mcp = is_mcp_jsonrpc_path(req.uri().path());

    // The workspace budget is not taken here. It is taken after authentication,
    // and only for a caller authenticated into that workspace
    // (`enforce_workspace`). Taking it from the path alone let any client who
    // knew the id spend it.
    if let Some(cfg) = global {
        // A bearer on a route that will resolve it is counted there: the
        // verified secret keeps its own bucket, and a secret that does not
        // resolve shares the client IP. Counting the raw secret here gave
        // every invented bearer a fresh bucket.
        if !defer_global_bearer(&state, &req) {
            let client = if state.auth_disabled {
                client_key(&req)
            } else {
                ip_client_key(&req)
            };
            let key = format!("global:{client}");
            if !try_acquire(&key, cfg.into(), redis.as_ref()).await {
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
        // Structured backpressure for MCP JSON-RPC clients: a JSON-RPC error
        // envelope with `retry_after_ms` in `data`, still under a 429 so
        // HTTP-level infra sees it too.
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

    #[test]
    fn forwarded_for_is_ignored_until_proxy_hops_are_declared() {
        let peer: IpAddr = "203.0.113.9".parse().expect("peer");
        assert_eq!(
            forwarded_client_ip("198.51.100.4, 203.0.113.8", peer, 0),
            Some(peer)
        );
        assert_eq!(
            forwarded_client_ip("198.51.100.4, 203.0.113.8", peer, 1),
            Some("203.0.113.8".parse().expect("proxy"))
        );
        assert_eq!(
            forwarded_client_ip("198.51.100.4, 203.0.113.8", peer, 2),
            Some("198.51.100.4".parse().expect("client"))
        );
    }

    #[test]
    fn malformed_or_short_forwarded_chain_falls_back_to_peer() {
        let peer: IpAddr = "203.0.113.9".parse().expect("peer");
        assert_eq!(forwarded_client_ip("garbage", peer, 1), None);
        assert_eq!(forwarded_client_ip("198.51.100.4", peer, 2), None);
    }

    /// The rules both limits share, checked on the raw values so the test
    /// does not race other lib tests over the process environment.
    fn assert_default_rules(spec: &LimitSpec, default_max: u32, default_window_secs: u64) {
        // Unset: off unless the bootstrap enabled the default.
        assert!(resolve(spec, None, None, false).is_none());
        let d = resolve(spec, None, None, true).expect("default floor when default_on");
        assert_eq!(d.max, default_max);
        assert_eq!(d.window, Duration::from_secs(default_window_secs));
        // A window alone does not change the default.
        let d = resolve(spec, None, Some("5"), true).expect("default");
        assert_eq!(d.window, Duration::from_secs(default_window_secs));

        // Explicit value wins regardless of the flag.
        assert_eq!(
            resolve(spec, Some("5"), None, false).map(|c| c.max),
            Some(5)
        );
        assert_eq!(resolve(spec, Some("5"), None, true).map(|c| c.max), Some(5));
        assert_eq!(
            resolve(spec, Some("5"), Some("10"), true).map(|c| c.window),
            Some(Duration::from_secs(10))
        );

        // Explicit 0, or junk, disables even with the default on.
        assert!(resolve(spec, Some("0"), None, true).is_none());
        assert!(resolve(spec, Some("lots"), None, true).is_none());
    }

    #[test]
    fn default_on_applies_a_floor_and_explicit_env_overrides() {
        assert_default_rules(&GLOBAL, DEFAULT_GLOBAL_MAX, DEFAULT_GLOBAL_WINDOW_SECS);
    }

    #[test]
    fn workspace_limit_has_a_default_on_the_global_limits_terms() {
        assert_default_rules(
            &WORKSPACE,
            DEFAULT_WORKSPACE_MAX,
            DEFAULT_WORKSPACE_WINDOW_SECS,
        );
    }

    #[test]
    fn workspace_budget_requires_that_workspaces_credential() {
        let own = maidan_types::WorkspaceId::new();
        let other = maidan_types::WorkspaceId::new();
        let path = format!("/workspaces/{own}/search");
        assert!(authenticated_into_path(false, &own, &path));
        assert!(!authenticated_into_path(false, &other, &path));
        assert!(authenticated_into_path(true, &other, &path));
        assert!(!authenticated_into_path(false, &own, "/workspaces"));
        assert!(!authenticated_into_path(true, &own, "/channels/x"));
    }

    #[test]
    fn invented_bearer_is_not_deferred_off_the_auth_routers() {
        assert!(bearer_resolved_later(
            &Method::GET,
            "/workspaces/abc/search"
        ));
        assert!(bearer_resolved_later(&Method::POST, "/mcp"));
        assert!(bearer_resolved_later(
            &Method::GET,
            "/ui/api/workspaces/abc/channels"
        ));
        assert!(bearer_resolved_later(&Method::POST, "/a2a/v1/events"));
        assert!(bearer_resolved_later(
            &Method::POST,
            "/auth/session/from-token"
        ));
        assert!(!bearer_resolved_later(&Method::POST, "/workspaces"));
        assert!(!bearer_resolved_later(
            &Method::POST,
            "/workspaces/abc/members"
        ));
        assert!(bearer_resolved_later(
            &Method::GET,
            "/workspaces/abc/members"
        ));
        assert!(!bearer_resolved_later(&Method::GET, "/openapi.json"));
        assert!(!bearer_resolved_later(&Method::GET, "/health/ready"));
        assert!(!bearer_resolved_later(&Method::GET, "/ui"));
        assert!(!bearer_resolved_later(&Method::GET, "/no-such"));
    }
}
