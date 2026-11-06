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
    http::{header, HeaderMap, HeaderValue, Method},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::Ipv6Addr;

use crate::{error::ApiError, AppState};

pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if rides_an_open_mode(&state, &req)
        && let Err(reason) = judge(&state.allowed_hosts, req.headers())
    {
        return ApiError::Forbidden(reason.into()).into_response();
    }
    next.run(req).await
}

/// Whether the request would be served without a credential.
///
/// Uses [`crate::auth::bearer_from_headers`] so an empty or non-Bearer
/// `Authorization` (which still makes `contains_key` true) cannot skip the
/// rebinding judge on the anonymous MCP reader path.
fn rides_an_open_mode(state: &AppState, req: &Request) -> bool {
    if state.auth_disabled {
        return true;
    }
    state.dev_anonymous_reader.is_some()
        && req.method() == Method::POST
        && crate::rate_limit::is_mcp_jsonrpc_path(req.uri().path())
        && crate::auth::bearer_from_headers(req.headers()).is_none()
}

/// `Err` with the reason a credential-less request is refused.
pub(crate) fn judge(allowed_hosts: &[String], headers: &HeaderMap) -> Result<(), &'static str> {
    // Duplicate Host headers are ambiguous; refuse rather than pick one.
    let host_values: Vec<&HeaderValue> = headers.get_all(header::HOST).iter().collect();
    if host_values.len() > 1 {
        return Err("a request with no credential must carry a single Host header");
    }
    let host_raw = match host_values.first() {
        None => "",
        Some(value) => value
            .to_str()
            .map_err(|_| "a request with no credential must carry a UTF-8 Host header")?,
    };
    let (name, host_port) = parse_host_authority(host_raw)
        .ok_or("a request with no credential must carry a well-formed Host header")?;
    let named = name.is_empty()
        || !name.contains('.')
        || is_loopback_name(&name)
        || name.parse::<std::net::IpAddr>().is_ok()
        || allowed_hosts.contains(&name);
    if !named {
        return Err(
            "a request with no credential must name this server: a loopback name, a name without a dot, an IP address, or a host in MAIDAN_ALLOWED_HOSTS",
        );
    }
    for origin in headers.get_all(header::ORIGIN) {
        let origin_str = origin
            .to_str()
            .map_err(|_| "a request from another origin needs a credential")?;
        let Some((origin_name, origin_port)) = parse_origin_authority(origin_str) else {
            return Err("a request from another origin needs a credential");
        };
        if !origin_matches_host(&name, host_port, &origin_name, origin_port) {
            return Err("a request from another origin needs a credential");
        }
    }
    Ok(())
}

fn origin_matches_host(
    host_name: &str,
    host_port: Option<u16>,
    origin_name: &str,
    origin_port: Option<u16>,
) -> bool {
    if host_name != origin_name {
        return false;
    }
    match (host_port, origin_port) {
        (None, None) => true,
        (Some(a), Some(b)) => a == b,
        // A Host without an explicit port matches an Origin that also omitted
        // the port (already handled) or — for string-equality compatibility
        // with historical callers — when the Origin authority carried no port
        // component either. Mixed explicit/default is a mismatch.
        (None, Some(_)) | (Some(_), None) => false,
    }
}

/// The names `MAIDAN_ALLOWED_HOSTS` lists (comma-separated, without ports),
/// lowercased.
pub fn allowed_hosts_from(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect()
}

/// Parse `Host` into a normalized (no trailing dot, lowercased) name and an
/// optional port. Rejects bracketed IPv6 with a trailing hostname suffix
/// (`[::1].evil.example.com`), multiple colons outside brackets, and
/// non-numeric ports.
fn parse_host_authority(authority: &str) -> Option<(String, Option<u16>)> {
    if authority.is_empty() {
        return Some((String::new(), None));
    }
    let (name, port) = split_host_port(authority)?;
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    Some((name, port))
}

fn split_host_port(authority: &str) -> Option<(&str, Option<u16>)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (addr, suffix) = rest.split_once(']')?;
        // Bracketed content must be a real IPv6 address.
        addr.parse::<Ipv6Addr>().ok()?;
        if suffix.is_empty() {
            return Some((addr, None));
        }
        let port = suffix.strip_prefix(':')?;
        // Reject `[::1].evil…` and `[::1]:` / non-u16 ports.
        if port.is_empty() || port.contains(':') {
            return None;
        }
        let port: u16 = port.parse().ok()?;
        return Some((addr, Some(port)));
    }
    match authority.rsplit_once(':') {
        Some((name, port)) if !name.is_empty() && !name.contains(':') => {
            if port.is_empty() {
                return None;
            }
            let port: u16 = port.parse().ok()?;
            Some((name, Some(port)))
        }
        // No port, or bare IPv6 without brackets (reject — ambiguous).
        None => {
            if authority.contains(':') {
                return None;
            }
            Some((authority, None))
        }
        Some(_) => None,
    }
}

fn parse_origin_authority(origin: &str) -> Option<(String, Option<u16>)> {
    let (_, authority) = origin.split_once("://")?;
    // Drop any path/userinfo that a non-browser might send.
    let authority = authority.split_once('/').map_or(authority, |(a, _)| a);
    if authority.is_empty() {
        return None;
    }
    parse_host_authority(authority)
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

    #[test]
    fn trailing_dot_hosts_normalize_for_origin_and_allowlist() {
        let allowed = allowed_hosts_from("app.example.com.");
        assert!(judge(
            &allowed,
            &headers("app.example.com.", Some("https://app.example.com"))
        )
        .is_ok());
        assert!(judge(
            &allowed,
            &headers("app.example.com", Some("https://app.example.com."))
        )
        .is_ok());
    }

    #[test]
    fn bracketed_ipv6_rejects_hostname_suffix_and_bad_ports() {
        assert!(parse_host_authority("[::1].evil.example.com:8080").is_none());
        assert!(parse_host_authority("[::1]:").is_none());
        assert!(parse_host_authority("[::1]:65536").is_none());
        assert!(parse_host_authority("[gggg::1]:8080").is_none());
        assert_eq!(
            parse_host_authority("[::1]:8080"),
            Some(("::1".into(), Some(8080)))
        );
    }

    #[test]
    fn duplicate_or_non_utf8_host_is_refused() {
        let mut dup = HeaderMap::new();
        dup.append(header::HOST, HeaderValue::from_static("localhost"));
        dup.append(header::HOST, HeaderValue::from_static("evil.example.com"));
        assert!(judge(&[], &dup).is_err());

        let mut bad = HeaderMap::new();
        bad.insert(
            header::HOST,
            HeaderValue::from_bytes(b"local\xffhost").unwrap(),
        );
        assert!(judge(&[], &bad).is_err());
    }

    #[test]
    fn every_origin_header_must_match_host() {
        let allowed = allowed_hosts_from("maidan.example.com");
        let mut headers = headers("maidan.example.com", Some("https://maidan.example.com"));
        headers.append(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example.com"),
        );
        assert!(judge(&allowed, &headers).is_err());
    }

    #[test]
    fn empty_authorization_still_rides_open_mode_check_via_bearer_parser() {
        // `Authorization: ` / non-Bearer must parse as no bearer so the
        // anonymous MCP path is still judged for rebinding.
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static(""));
        assert!(crate::auth::bearer_from_headers(&headers).is_none());
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic x"));
        assert!(crate::auth::bearer_from_headers(&headers).is_none());
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer tok"),
        );
        assert_eq!(crate::auth::bearer_from_headers(&headers), Some("tok"));
    }
}
