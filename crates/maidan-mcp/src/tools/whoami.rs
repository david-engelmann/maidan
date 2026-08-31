//! Agent self-discovery (Cluster 336). Every hero-loop tool needs the caller's
//! own `member_id`; `whoami` returns it (plus workspace + capabilities) so an
//! agent handed only a base URL + token can bootstrap without an out-of-band
//! lookup. Reflects the request's `AuthContext` — no store access.

use maidan_auth::AuthContext;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

pub(super) async fn whoami(auth: &AuthContext) -> Result<Value, McpError> {
    Ok(content_json(&json!({
        "member_id": auth.member_id.0,
        "workspace_id": auth.workspace_id.0,
        "capabilities": auth.capabilities(),
        // A bearer token acts as any member (orchestrator model); a browser/OIDC
        // session is pinned to its own member. `bypass` = auth disabled (dev).
        "is_bearer": auth.token_id.is_some(),
        "bypass": auth.bypass,
    })))
}
