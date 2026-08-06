//! Capability strings for v0.5.0. Stored on API tokens as JSON arrays.

pub const WORKSPACE_READ: &str = "workspace:read";
pub const WORKSPACE_WRITE: &str = "workspace:write";
pub const MESSAGE_POST: &str = "message:post";
pub const THREAD_TRANSITION: &str = "thread:transition";
pub const ARTIFACT_UPLOAD: &str = "artifact:upload";
pub const SEARCH_QUERY: &str = "search:query";
pub const EVENT_SUBSCRIBE: &str = "event:subscribe";
pub const TOKEN_ADMIN: &str = "token:admin";
pub const FEDERATION_INGEST: &str = "federation:ingest";
pub const FEDERATION_ADMIN: &str = "federation:admin";
/// Read the audit log across **all** workspaces (operator/global; Cluster 132).
/// Not workspace-scoped — a token holding it queries `/operator/audit` without
/// an `ensure_workspace` check.
pub const AUDIT_READ_GLOBAL: &str = "audit:read-global";
/// Manage per-channel membership (add/remove/list `channel_members`; Cluster
/// 164). Deliberately not in [`default_minted`] — channel administration is a
/// granted-on-purpose surface.
pub const CHANNEL_ADMIN: &str = "channel:admin";

const KNOWN: &[&str] = &[
    WORKSPACE_READ,
    WORKSPACE_WRITE,
    MESSAGE_POST,
    THREAD_TRANSITION,
    ARTIFACT_UPLOAD,
    SEARCH_QUERY,
    EVENT_SUBSCRIBE,
    TOKEN_ADMIN,
    FEDERATION_INGEST,
    FEDERATION_ADMIN,
    AUDIT_READ_GLOBAL,
    CHANNEL_ADMIN,
];

/// Default capabilities for tokens minted by the admin API in tests and docs.
pub fn default_minted() -> Vec<String> {
    vec![
        WORKSPACE_READ.into(),
        WORKSPACE_WRITE.into(),
        EVENT_SUBSCRIBE.into(),
        SEARCH_QUERY.into(),
    ]
}

pub fn is_known(cap: &str) -> bool {
    KNOWN.contains(&cap)
}

pub fn validate_list(caps: &[String]) -> Result<(), String> {
    for cap in caps {
        if !is_known(cap) {
            return Err(format!("unknown capability: {cap}"));
        }
    }
    Ok(())
}

/// Ensures every requested capability is included in the installation grant.
pub fn validate_subset(granted: &[String], requested: &[String]) -> Result<(), String> {
    validate_list(requested)?;
    for cap in requested {
        if !granted.iter().any(|g| g == cap) {
            return Err(format!("capability {cap} exceeds app installation grant"));
        }
    }
    Ok(())
}
