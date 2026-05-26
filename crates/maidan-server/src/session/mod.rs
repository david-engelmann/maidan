//! Browser session cookie helpers and request context.

mod cookie;
mod handlers;
pub mod middleware;

pub use cookie::session_secret_from_env;

use axum::http::{header, HeaderMap, HeaderValue};
use maidan_types::{MemberId, SessionId, WorkspaceId};

pub use handlers::{get_session, mint_first_admin_token};
pub use middleware::{load_session, require_middleware};

pub const SESSION_COOKIE: &str = "maidan_session";

#[derive(Debug, Clone)]
pub struct SessionContext {
    pub session_id: SessionId,
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
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
