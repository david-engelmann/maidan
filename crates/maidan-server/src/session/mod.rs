//! Browser session cookie helpers and request context.

mod cookie;
mod handlers;
pub mod middleware;

pub use cookie::session_secret_from_env;

use std::sync::Arc;

use axum::http::{header, HeaderMap, HeaderValue, Method};
use maidan_auth::AuthContext;
use maidan_types::{MemberId, SessionId, WorkspaceId};

use crate::config::ConfigError;
use crate::error::ApiError;

pub use handlers::{
    get_session, list_session_workspaces, mint_first_admin_token, session_from_token,
    SESSION_WORKSPACES_LIMIT,
};
pub use middleware::{load_session, require_middleware};

pub const SESSION_COOKIE: &str = "maidan_session";

const DEFAULT_SESSION_TTL_SECS: u64 = 28_800;

#[derive(Debug, Clone)]
pub struct SessionContext {
    pub session_id: SessionId,
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    /// The authority of the token this session was made from, resolved again
    /// for this request. `None` for an OIDC session.
    pub token: Option<AuthContext>,
}

impl SessionContext {
    /// The authority this request carries: its token's, or for an OIDC
    /// session the fixed `oidc_capabilities` of the routes it is on.
    pub fn auth_context(&self, oidc_capabilities: &[&str]) -> AuthContext {
        match &self.token {
            Some(ctx) => ctx.clone(),
            None => AuthContext::from_session(
                self.member_id,
                self.workspace_id,
                oidc_capabilities.iter().map(|c| (*c).to_string()).collect(),
            ),
        }
    }
}

/// How browser sessions are signed and how long they last. OIDC login and the
/// token exchange share them.
#[derive(Clone)]
pub struct SessionSettings {
    pub secret: Arc<[u8]>,
    pub ttl_secs: u64,
    pub cookie_secure: bool,
}

impl SessionSettings {
    /// `None` when `MAIDAN_SESSION_SECRET` is unset: with no key there are no
    /// sessions, and the token exchange answers 404.
    pub fn from_env() -> Result<Option<Self>, ConfigError> {
        if std::env::var_os("MAIDAN_SESSION_SECRET").is_none() {
            return Ok(None);
        }
        let ttl_secs = std::env::var("MAIDAN_SESSION_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_SESSION_TTL_SECS);
        let cookie_secure = matches!(
            std::env::var("MAIDAN_COOKIE_SECURE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ) || std::env::var("MAIDAN_ENV").as_deref() == Ok("production");
        Ok(Some(Self {
            secret: session_secret_from_env()?,
            ttl_secs,
            cookie_secure,
        }))
    }
}

/// Refuse an unsafe request that a session cookie authenticates when a browser
/// sent it from another origin. A bearer is never sent by the browser on its
/// own, so only the cookie needs this.
///
/// `SameSite=Lax` already keeps the cookie off cross-site POSTs; this closes
/// the gap it leaves, a sibling origin on the same site (another subdomain or
/// port), which Lax treats as same-site.
pub fn check_request_origin(method: &Method, headers: &HeaderMap) -> Result<(), ApiError> {
    if matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    ) {
        return Ok(());
    }
    refuse_cross_origin(headers)
}

/// The origin test itself, for any method: a WebSocket handshake is a `GET`
/// whose stream the other origin could read, so it takes this directly.
///
/// `Sec-Fetch-Site` is the browser's own verdict and survives a proxy that
/// rewrites `Host`, so it decides when present. Without it, `Origin` must name
/// this host. With neither, the request is accepted. That is a fallback for a
/// client that is not a browser and already holds the cookie. It is not a
/// guarantee that a page in another origin cannot omit both headers.
pub fn refuse_cross_origin(headers: &HeaderMap) -> Result<(), ApiError> {
    let refused =
        || ApiError::Forbidden("a cross-origin request cannot use a browser session".into());
    if let Some(site) = headers.get("sec-fetch-site") {
        return match site.to_str() {
            Ok("same-origin") | Ok("none") => Ok(()),
            _ => Err(refused()),
        };
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    let origin_authority = origin
        .to_str()
        .ok()
        .and_then(|o| o.split_once("://"))
        .map(|(_, authority)| authority)
        .ok_or_else(refused)?;
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .ok_or_else(refused)?;
    if origin_authority.eq_ignore_ascii_case(host) {
        Ok(())
    } else {
        Err(refused())
    }
}

/// The origin test a session request must pass to accept an approval gate.
/// Stricter than [`refuse_cross_origin`]: a request that names no origin at
/// all is refused, because the fallback that lets a non-browser client holding
/// the cookie through is exactly the client an acceptance must not come from.
/// `Sec-Fetch-Site` decides when present and must be `same-origin`; without
/// it, `Origin` must name this host.
pub fn require_same_origin(headers: &HeaderMap) -> Result<(), ApiError> {
    let refused = || {
        ApiError::Forbidden(
            "accepting an approval gate from a browser session needs the request to come \
             from the console page: it named no matching origin"
                .into(),
        )
    };
    if let Some(site) = headers.get("sec-fetch-site") {
        return match site.to_str() {
            Ok("same-origin") => Ok(()),
            _ => Err(refused()),
        };
    }
    if headers.get(header::ORIGIN).is_none() {
        return Err(refused());
    }
    refuse_cross_origin(headers).map_err(|_| refused())
}

pub fn parse_session_cookie(headers: &HeaderMap, secret: &[u8]) -> Option<SessionId> {
    let raw = headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{SESSION_COOKIE}=")))?;
    cookie::verify_session_value(raw, secret)
}

pub fn set_session_cookie(
    headers: &mut HeaderMap,
    session_id: SessionId,
    max_age_secs: u64,
    secure: bool,
    secret: &[u8],
) -> Result<(), header::InvalidHeaderValue> {
    let signed = cookie::sign_session_value(session_id, secret);
    let mut value = format!(
        "{SESSION_COOKIE}={signed}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}"
    );
    if secure {
        value.push_str("; Secure");
    }
    headers.append(header::SET_COOKIE, HeaderValue::from_str(&value)?);
    Ok(())
}

pub fn clear_session_cookie(
    headers: &mut HeaderMap,
    secure: bool,
) -> Result<(), header::InvalidHeaderValue> {
    let mut value = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    if secure {
        value.push_str("; Secure");
    }
    headers.append(header::SET_COOKIE, HeaderValue::from_str(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (name, value) in pairs {
            h.insert(*name, HeaderValue::from_static(value));
        }
        h
    }

    #[test]
    fn a_session_write_from_another_origin_is_refused() {
        let post = Method::POST;
        let same = headers(&[
            ("host", "maidan.example"),
            ("origin", "https://maidan.example"),
        ]);
        assert!(check_request_origin(&post, &same).is_ok());

        for other in [
            headers(&[
                ("host", "maidan.example"),
                ("origin", "https://evil.example"),
            ]),
            headers(&[
                ("host", "maidan.example"),
                ("origin", "https://sub.maidan.example"),
            ]),
            headers(&[
                ("host", "maidan.example:443"),
                ("origin", "https://maidan.example:8443"),
            ]),
            headers(&[("host", "maidan.example"), ("origin", "null")]),
            headers(&[("sec-fetch-site", "same-site")]),
            headers(&[("sec-fetch-site", "cross-site")]),
            headers(&[
                ("sec-fetch-site", "cross-site"),
                ("host", "maidan.example"),
                ("origin", "https://maidan.example"),
            ]),
        ] {
            assert!(check_request_origin(&post, &other).is_err(), "{other:?}");
        }
    }

    #[test]
    fn the_browsers_own_verdict_and_non_browser_clients_are_trusted() {
        let delete = Method::DELETE;
        // A proxy rewrote Host, but the browser says the page is this origin.
        let proxied = headers(&[
            ("sec-fetch-site", "same-origin"),
            ("host", "backend:8080"),
            ("origin", "https://maidan.example"),
        ]);
        assert!(check_request_origin(&delete, &proxied).is_ok());
        // No Origin and no Sec-Fetch-Site: accepted, not an origin guarantee.
        assert!(check_request_origin(&delete, &HeaderMap::new()).is_ok());
    }

    #[test]
    fn accepting_a_gate_needs_a_positive_same_origin_signal() {
        for same in [
            headers(&[("sec-fetch-site", "same-origin")]),
            headers(&[
                ("host", "maidan.example"),
                ("origin", "https://maidan.example"),
            ]),
        ] {
            assert!(require_same_origin(&same).is_ok(), "{same:?}");
        }
        for other in [
            HeaderMap::new(),
            headers(&[("host", "maidan.example")]),
            headers(&[("sec-fetch-site", "none")]),
            headers(&[("sec-fetch-site", "same-site")]),
            headers(&[
                ("sec-fetch-site", "cross-site"),
                ("host", "maidan.example"),
                ("origin", "https://maidan.example"),
            ]),
            headers(&[
                ("host", "maidan.example"),
                ("origin", "https://evil.example"),
            ]),
        ] {
            assert!(require_same_origin(&other).is_err(), "{other:?}");
        }
    }

    #[test]
    fn a_safe_method_is_not_origin_checked_but_a_handshake_can_be() {
        let other = headers(&[("sec-fetch-site", "cross-site")]);
        assert!(check_request_origin(&Method::GET, &other).is_ok());
        assert!(refuse_cross_origin(&other).is_err());
    }
}
