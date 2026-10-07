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
//! A rebinding page presents a public name it controls, which always has a
//! dot. So a request that rides one of those open modes is refused (403) when
//! its `Host` is a dotted name that is neither a loopback name nor listed in
//! `MAIDAN_ALLOWED_HOSTS`, and when it carries an `Origin` whose authority is
//! not its `Host`. An IP address, a name without a dot (a compose service such
//! as `maidan`) and a request with no `Host` at all (no browser sends one) are
//! not rebound names. The rule holds however the server is bound, including on
//! every interface as a container is.
//!
//! For the anonymous reader, a request that carries a credential is not judged
//! here: it takes the bearer path, which refuses a token it does not know, so a
//! reverse proxy that forwards a public `Host` keeps working. Under
//! `AUTH_DISABLED` every request is judged, because no credential is checked and
//! a page can attach any header; behind such a proxy, list its name in
//! `MAIDAN_ALLOWED_HOSTS`.

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, Method},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{error::ApiError, AppState};

pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if rides_an_open_mode(&state, &req) {
        if let Err(reason) = judge(&state.allowed_hosts, req.headers()) {
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
pub(crate) fn judge(allowed_hosts: &[String], headers: &HeaderMap) -> Result<(), &'static str> {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let name = host_name(host).trim_end_matches('.').to_ascii_lowercase();
    let named = name.is_empty()
        || !name.contains('.')
        || is_loopback_name(&name)
        || name.parse::<std::net::IpAddr>().is_ok()
        || allowed_hosts.contains(&name);
    if !named {
        return Err("a request with no credential must name this server: a loopback name, a name without a dot, an IP address, or a host in MAIDAN_ALLOWED_HOSTS");
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

/// The names `MAIDAN_ALLOWED_HOSTS` lists (comma-separated, without ports),
/// lowercased.
pub fn allowed_hosts_from(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect()
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
    fn a_rebound_name_is_refused_however_the_server_is_bound() {
        let rebound = headers(
            "evil.example.com:8080",
            Some("http://evil.example.com:8080"),
        );
        assert!(judge(&[], &rebound).is_err());
        assert!(
            judge(&[], &HeaderMap::new()).is_ok(),
            "no browser sends no Host"
        );
        for host in [
            "localhost:8080",
            "127.0.0.1:8080",
            "[::1]:8080",
            "app.localhost",
            "127.0.0.2",
            "192.168.1.20:8080",
            "[fd00::1]:8080",
            "maidan:8080",
        ] {
            let origin = format!("http://{host}");
            assert!(judge(&[], &headers(host, Some(&origin))).is_ok(), "{host}");
            assert!(judge(&[], &headers(host, None)).is_ok(), "{host}");
        }
    }

    #[test]
    fn a_listed_host_is_this_server() {
        let allowed = allowed_hosts_from(" Dev.Example.com , ,other.example.");
        assert_eq!(allowed, ["dev.example.com", "other.example"]);
        let named = headers("dev.example.com", Some("https://dev.example.com"));
        assert!(judge(&allowed, &named).is_ok());
        assert!(judge(&[], &named).is_err());
    }

    #[test]
    fn another_origin_is_refused_wherever_the_server_is() {
        let allowed = allowed_hosts_from("maidan.example.com");
        let cross = headers("maidan.example.com", Some("https://other.example.com"));
        assert!(judge(&allowed, &cross).is_err());
        assert!(judge(
            &[],
            &headers("localhost:8080", Some("http://localhost:9999"))
        )
        .is_err());
        assert!(judge(&allowed, &headers("maidan.example.com", Some("null"))).is_err());
        assert!(judge(&allowed, &headers("maidan.example.com", None)).is_ok());
        assert!(judge(
            &allowed,
            &headers("maidan.example.com", Some("https://maidan.example.com"))
        )
        .is_ok());
    }
}
