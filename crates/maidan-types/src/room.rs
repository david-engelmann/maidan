//! Room handle + discovery document.
//!
//! A workspace is a **room**. Its stored id is the workspace UUID. A handle is
//! an optional, renameable alias — never an id. Stored [`crate::RoomUri`]
//! values use the UUID, so a handle rename cannot break a citation, pin, or
//! export.
//!
//! The handle lives on a separate table (not a `Workspace` column) so
//! `row_to_workspace` does not ripple. Syntax is validated here; uniqueness is
//! the store's job.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::ids::WorkspaceId;
use crate::room_uri::{RoomUri, ROOM_URI_TEMPLATE};

/// Observable `$type` for a per-workspace room card. Breaking = `/2`.
pub const ROOM_TYPE: &str = "maidan.room/1";

/// Observable `$type` for the instance discovery document at
/// `/.well-known/maidan-room`. No tenant list — handles are aliases,
/// not a public directory.
pub const ROOM_DISCOVERY_TYPE: &str = "maidan.room-discovery/1";

/// Max handle length (DNS-label-ish; not a UUID).
pub const WORKSPACE_HANDLE_MAX: usize = 64;

/// A workspace's current handle alias. The workspace id is the stored
/// identity; this row is the renameable name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceHandle {
    pub workspace_id: WorkspaceId,
    pub handle: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Authenticated room card: stable id + current handle + canonical URI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoomCard {
    #[serde(rename = "$type")]
    pub type_id: String,
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    pub uri: String,
}

impl RoomCard {
    pub fn new(workspace_id: WorkspaceId, handle: Option<String>) -> Self {
        Self {
            type_id: ROOM_TYPE.to_string(),
            workspace_id,
            handle,
            uri: RoomUri::workspace(workspace_id).to_string(),
        }
    }
}

/// Public `/.well-known/maidan-room` document. Scheme only — no
/// workspace list (that would leak tenants).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoomDiscovery {
    #[serde(rename = "$type")]
    pub type_id: String,
    pub uri_template: String,
    pub id_kind: String,
    pub handle_is_alias: bool,
}

impl RoomDiscovery {
    pub fn document() -> Self {
        Self {
            type_id: ROOM_DISCOVERY_TYPE.to_string(),
            uri_template: ROOM_URI_TEMPLATE.to_string(),
            id_kind: "uuid".into(),
            handle_is_alias: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RoomHandleError {
    #[error("workspace handle must be 1..={WORKSPACE_HANDLE_MAX} characters")]
    Length,
    #[error("workspace handle must start with a lowercase letter and contain only [a-z0-9-]")]
    Syntax,
    #[error("workspace handle must not be a UUID — ids are stored separately")]
    LooksLikeId,
}

/// Validate a workspace handle. Rejects UUIDs so a handle can never be
/// confused with the room URI authority.
pub fn validate_workspace_handle(handle: &str) -> Result<(), RoomHandleError> {
    if handle.is_empty() || handle.len() > WORKSPACE_HANDLE_MAX {
        return Err(RoomHandleError::Length);
    }
    let mut chars = handle.chars();
    let Some(first) = chars.next() else {
        return Err(RoomHandleError::Length);
    };
    if !first.is_ascii_lowercase() {
        return Err(RoomHandleError::Syntax);
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(RoomHandleError::Syntax);
    }
    if handle.ends_with('-') || handle.contains("--") {
        return Err(RoomHandleError::Syntax);
    }
    if Uuid::parse_str(handle).is_ok() {
        return Err(RoomHandleError::LooksLikeId);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_document_is_scheme_only() {
        let doc = RoomDiscovery::document();
        assert_eq!(doc.type_id, ROOM_DISCOVERY_TYPE);
        assert_eq!(doc.uri_template, ROOM_URI_TEMPLATE);
        assert_eq!(doc.id_kind, "uuid");
        assert!(doc.handle_is_alias);
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(json["$type"], ROOM_DISCOVERY_TYPE);
        assert!(json.get("rooms").is_none(), "no tenant list");
        assert!(json.get("workspace_id").is_none(), "no tenant id");
    }

    #[test]
    fn room_card_uri_uses_workspace_id_not_handle() {
        let id = WorkspaceId(Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap());
        let card = RoomCard::new(id, Some("acme".into()));
        assert_eq!(card.type_id, ROOM_TYPE);
        assert_eq!(card.handle.as_deref(), Some("acme"));
        assert_eq!(card.uri, "maidan://11111111-1111-4111-8111-111111111111");
        assert!(!card.uri.contains("acme"));
    }

    #[test]
    fn handle_rename_does_not_change_the_card_uri() {
        let id = WorkspaceId(Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap());
        let before = RoomCard::new(id, Some("acme".into()));
        let after = RoomCard::new(id, Some("renamed".into()));
        assert_eq!(before.uri, after.uri);
        assert_ne!(before.handle, after.handle);
    }

    #[test]
    fn valid_handles_are_accepted() {
        for h in ["acme", "a", "team-1", "ok-ok", "a1b2"] {
            validate_workspace_handle(h).unwrap_or_else(|e| panic!("{h}: {e}"));
        }
    }

    #[test]
    fn invalid_handles_are_rejected() {
        assert_eq!(
            validate_workspace_handle("").unwrap_err(),
            RoomHandleError::Length
        );
        assert_eq!(
            validate_workspace_handle(&"a".repeat(65)).unwrap_err(),
            RoomHandleError::Length
        );
        assert_eq!(
            validate_workspace_handle("Acme").unwrap_err(),
            RoomHandleError::Syntax
        );
        assert_eq!(
            validate_workspace_handle("1acme").unwrap_err(),
            RoomHandleError::Syntax
        );
        assert_eq!(
            validate_workspace_handle("-acme").unwrap_err(),
            RoomHandleError::Syntax
        );
        assert_eq!(
            validate_workspace_handle("acme-").unwrap_err(),
            RoomHandleError::Syntax
        );
        assert_eq!(
            validate_workspace_handle("ac--me").unwrap_err(),
            RoomHandleError::Syntax
        );
        assert_eq!(
            validate_workspace_handle("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap_err(),
            RoomHandleError::LooksLikeId
        );
    }
}
