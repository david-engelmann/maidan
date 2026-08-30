//! Context-snapshot MCP tool (Cluster 330) — the twin of the REST route
//! (Cluster 329). Freezes the assembled context pack (live or `as_of`) into the
//! content-addressed artifact store: a tamper-evident, deduped record of exactly
//! what the agent was handed. Reuses the shared `context::get_thread_context`
//! builder, then the modern `*_with_event` upsert + a bus-notify.

use bytes::Bytes;
use maidan_auth::AuthContext;
use maidan_types::*;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;
use crate::server::McpServer;

pub(super) async fn snapshot_thread_context(
    server: &McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    // Build the pack via the shared context builder (honors as_of / include_*).
    let pack = crate::context::get_thread_context(store.as_ref(), args).await?;
    let raw = serde_json::to_vec(&pack)
        .map_err(|e| McpError::Internal(format!("serialize context snapshot: {e}")))?;
    let size_bytes = raw.len() as i64;
    let sha = server
        .artifacts
        .put(Bytes::from(raw))
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    let ref_workspace = (!auth.bypass).then_some(auth.workspace_id);
    let uploaded_by = (!auth.bypass).then_some(auth.member_id);
    let (artifact, stored) = store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha.to_string(),
                size_bytes,
                mime_type: Some("application/json".to_string()),
                kind: ArtifactKind::ContextSnapshot,
                uploaded_by,
            },
            ref_workspace,
        )
        .await?;
    // Bus-notify the already-durably-appended event (the MCP analogue of REST
    // publish_stored); a missing bus (embedded use) is a no-op.
    if let Some(bus) = server.event_bus.as_ref() {
        if let Ok(event) = serde_json::from_value::<Event>(stored.payload.clone()) {
            let _ = bus
                .publish(BusEnvelope {
                    log_id: stored.id,
                    event,
                })
                .await;
        }
    }
    Ok(content_json(&artifact))
}
