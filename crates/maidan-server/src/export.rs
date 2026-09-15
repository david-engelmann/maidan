//! Workspace export / portability (Cluster 187 + signed envelope, Cluster 391).
//!
//! Assembles a workspace's collaboration graph into one JSON bundle so an
//! operator can migrate or archive a tenant. **Tokens die on export.**
//! Cluster 391 wraps the bundle in a signed `maidan.workspace.export/1`
//! envelope so a blank instance can verify it without calling the origin.

use std::sync::Arc;

use maidan_auth::{sign_export, verify_export, ExportSigningKey};
use maidan_store::{build_workspace_export, Store, StoreError};
use maidan_types::*;

use crate::error::ApiError;

pub use maidan_types::{
    ExportChannel, WorkspaceExport, WORKSPACE_EXPORT_FORMAT_VERSION as FORMAT_VERSION,
};

/// Read the whole workspace content graph (shared with MCP).
pub async fn build(
    store: &Arc<dyn Store>,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceExport, StoreError> {
    build_workspace_export(store.as_ref(), workspace_id).await
}

/// Sign the content graph. Refuses if the operator key is missing — never
/// emit an unsigned bundle.
pub fn sign_bundle(
    key: Option<&ExportSigningKey>,
    bundle: &WorkspaceExport,
) -> Result<SignedExport, ApiError> {
    let key = key.ok_or_else(|| {
        ApiError::BadRequest(
            "export signing key is not configured (MAIDAN_EXPORT_SIGNING_KEY)".into(),
        )
    })?;
    let payload = serde_json::to_value(bundle)
        .map_err(|e| ApiError::Internal(format!("export serialize: {e}")))?;
    sign_export(key, payload).map_err(|e| ApiError::BadRequest(e.to_string()))
}

/// Verify a signed envelope. Empty `expected` = integrity against the
/// embedded public key (blank GHCR instance). Non-empty = authenticity pin.
pub fn verify_bundle(envelope: &SignedExport, expected: &[[u8; 32]]) -> Result<(), ApiError> {
    let pin = if expected.is_empty() {
        None
    } else {
        Some(expected)
    };
    verify_export(envelope, pin).map_err(|e| ApiError::BadRequest(e.to_string()))
}

/// Decode the inner Cluster-187 graph after the signature has been checked.
pub fn inner_bundle(envelope: &SignedExport) -> Result<WorkspaceExport, ApiError> {
    serde_json::from_value(envelope.payload.clone())
        .map_err(|e| ApiError::BadRequest(format!("export payload is not a workspace bundle: {e}")))
}

pub fn payload_workspace_id(payload: &serde_json::Value) -> Option<uuid::Uuid> {
    payload
        .get("workspace")
        .and_then(|w| w.get("id"))
        .and_then(|id| id.as_str())
        .and_then(|s| s.parse().ok())
}
