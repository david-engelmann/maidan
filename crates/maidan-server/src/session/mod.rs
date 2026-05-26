//! Browser session cookie helpers and request context.

mod handlers;
pub mod middleware;

use axum::http::{header, HeaderMap, HeaderValue};
use maidan_types::{MemberId, SessionId, WorkspaceId};
use uuid::Uuid;

pub use handlers::{get_session, mint_first_admin_token};
pub use middleware::{load_session, require_middleware};

pub const SESSION_COOKIE: &str = "maidan_session";

#[derive(Debug, Clone)]
pub struct SessionContext {
    pub session_id: SessionId,
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
}

pub fn parse_session_cookie(headers: &HeaderMap) -> Option<SessionId> {
    let raw = headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{SESSION_COOKIE}=")))?;
    let id = Uuid::parse_str(raw).ok()?;
    Some(SessionId(id))
}

pub fn set_session_cookie(
    headers: &mut HeaderMap,
    session_id: SessionId,
    max_age_secs: u64,
    secure: bool,
) -> Result<(), header::InvalidHeaderValue> {
    let mut value = format!(
        "{SESSION_COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}",
        session_id.0
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
