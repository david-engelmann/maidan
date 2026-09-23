//! Agent self-discovery. Every hero-loop tool needs the caller's own
//! `member_id`; `whoami` returns it (plus workspace + capabilities) so an agent
//! handed only a base URL + token can bootstrap without an out-of-band lookup.
//! Reflects the request's `AuthContext` — no store access.

use maidan_auth::AuthContext;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

pub(super) async fn whoami(auth: &AuthContext) -> Result<Value, McpError> {
    Ok(content_json(&json!({
        "actor_id": auth.actor_id.0,
        "member_id": auth.member_id.0,
        "delegation_grant_id": auth.delegation_grant_id.map(|id| id.0),
        "workspace_id": auth.workspace_id.0,
        "capabilities": auth.capabilities(),
        "capability_sets": maidan_auth::held_sets(auth.capabilities()),
        // Bearer and session identities are authentication-bound. `bypass`
        // exists only for explicitly insecure development mode.
        "is_bearer": auth.token_id.is_some(),
        "bypass": auth.bypass,
    })))
}
