//! Refuse a DNS-rebinding request where one could matter.
//!
//! A page on another site can point its own name at `127.0.0.1` and then send
//! requests to a server on this machine, past the browser's same-origin rule,
//! with `Host` and `Origin` both naming the page's site. That gains it nothing
//! where a request needs a credential: the page holds no bearer token, and the
//! browser sends none of this server's cookies to the page's name. It gains a
//! request with no credential everything that request may do, which here means
//! `AUTH_DISABLED` and the anonymous dev reader of MCP.
//!
//! So a request that rides one of those open modes is refused (403) when the
//! server is bound to a loopback address and `Host` is not a loopback name, and
//! when it carries an `Origin` whose authority is not its `Host`. A request with
//! a credential is never judged here, so a reverse proxy that forwards a public
//! `Host` to a loopback-bound server keeps working.

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, Method},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{error::ApiError, AppState};

pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if rides_an_open_mode(&state, &req) {
        if let Err(reason) = judge(state.loopback_bind, req.headers()) {
            return ApiError::Forbidden(reason.into()).into_response();
        }
    }
    next.run(req).await
}

/// Whether the request would be served without a credential.
fn rides_an_open_mode(state: &AppState, req: &Request) -> bool {
    if state.auth_disabled {
        return true;
    }
    state.dev_anonymous_reader.is_some()
        && req.method() == Method::POST
        && crate::rate_limit::is_mcp_jsonrpc_path(req.uri().path())
        && !req.headers().contains_key(header::AUTHORIZATION)
}

/// `Err` with the reason a credential-less request is refused.
pub(crate) fn judge(loopback_bind: bool, headers: &HeaderMap) -> Result<(), &'static str> {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    if loopback_bind && !is_loopback_name(host_name(host)) {
        return Err("a server on a loopback address answers only a loopback Host");
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let authority = origin
            .to_str()
            .ok()
            .and_then(|o| o.split_once("://"))
            .map(|(_, authority)| authority);
        if !authority.is_some_and(|a| a.eq_ignore_ascii_case(host)) {
            return Err("a request from another origin needs a credential");
        }
    }
    Ok(())
}

/// `host[:port]` or `[v6][:port]`, without the port.
fn host_name(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split_once(']').map_or(rest, |(name, _)| name);
    }
    authority
        .rsplit_once(':')
        .map_or(authority, |(name, _)| name)
}

fn is_loopback_name(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    name == "localhost"
        || name.ends_with(".localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(host: &str, origin: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        headers
    }

    #[test]
    fn a_rebound_name_is_refused_on_a_loopback_server() {
        let rebound = headers(
            "evil.example.com:8080",
            Some("http://evil.example.com:8080"),
        );
        assert!(judge(true, &rebound).is_err());
        for host in [
            "localhost:8080",
            "127.0.0.1:8080",
            "[::1]:8080",
            "app.localhost",
            "127.0.0.2",
        ] {
            let origin = format!("http://{host}");
            assert!(judge(true, &headers(host, Some(&origin))).is_ok(), "{host}");
            assert!(judge(true, &headers(host, None)).is_ok(), "{host}");
        }
    }

    #[test]
    fn another_origin_is_refused_wherever_the_server_is_bound() {
        let cross = headers("maidan.example.com", Some("https://other.example.com"));
        assert!(judge(false, &cross).is_err());
        assert!(judge(
            true,
            &headers("localhost:8080", Some("http://localhost:9999"))
        )
        .is_err());
        assert!(judge(false, &headers("maidan.example.com", Some("null"))).is_err());
        assert!(judge(false, &headers("maidan.example.com", None)).is_ok());
        assert!(judge(
            false,
            &headers("maidan.example.com", Some("https://maidan.example.com"))
        )
        .is_ok());
    }
}
