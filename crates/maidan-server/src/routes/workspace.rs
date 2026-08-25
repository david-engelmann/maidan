//! Workspace lifecycle handlers: create/get, context, purge/erase, audit,
//! events, and the quarantined-outbox replay/list endpoints.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{AUDIT_READ_GLOBAL, TOKEN_ADMIN, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_store::StoreError;
use maidan_types::*;

#[cfg(feature = "bootstrap")]
use super::publish_stored;
use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::federation::PeerContext;
use crate::state::AppState;

#[cfg(feature = "bootstrap")]
pub async fn create_workspace(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CreateWorkspace>,
) -> ApiResult<(StatusCode, Json<Workspace>)> {
    if !state.auth_disabled && state.bootstrap_enabled {
        let count = state.store.count_workspaces().await?;
        if count > 0 {
            return Err(ApiError::Forbidden(
                "bootstrap only allows creating the first workspace; use bearer auth thereafter"
                    .into(),
            ));
        }
    }
    let (ws, stored) = state
        .store
        .create_workspace_with_event(NewWorkspace { name: body.name })
        .await?;
    publish_stored(&state, stored).await;
    Ok((StatusCode::CREATED, Json(ws)))
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Workspace>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.get_workspace(workspace_id).await?))
}

/// Export the whole workspace content graph as one JSON bundle (Cluster 187).
/// Gated on `token:admin` — it dumps every channel (private included) and DM,
/// so it's a workspace-admin operation, not a per-member read. Secrets and
/// operational tables are excluded (see [`crate::export`]).
pub async fn export_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<crate::export::WorkspaceExport>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        crate::export::build(&state.store, workspace_id).await?,
    ))
}

/// Import a workspace content bundle (Cluster 270) — the write-side inverse of
/// [`export_workspace`]. Gated on `token:admin` (it creates or overwrites a whole
/// workspace). `mode=new` (default) remaps every id and lands a fresh workspace;
/// `mode=restore` preserves the bundle's ids — 409 if that workspace already
/// exists, unless `force=true` erases it first.
pub async fn import_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<crate::dto::ImportQuery>,
    ApiJson(bundle): ApiJson<crate::export::WorkspaceExport>,
) -> ApiResult<Json<crate::dto::ImportResult>> {
    use crate::dto::ImportMode;
    cap(&auth, TOKEN_ADMIN)?;

    let flat = crate::import::flatten(bundle);
    let to_write = match q.mode {
        ImportMode::New => crate::import::remap(flat, uuid::Uuid::new_v4),
        ImportMode::Restore => {
            let existing = state.store.get_workspace(flat.workspace.id).await;
            match existing {
                Ok(_) if !q.force => {
                    return Err(ApiError::Conflict(format!(
                        "workspace {} already exists; retry with force=true to overwrite",
                        flat.workspace.id.0
                    )));
                }
                Ok(_) => {
                    // force: erase the existing workspace so the restore lands cleanly.
                    state.store.erase_workspace(flat.workspace.id).await?;
                }
                Err(StoreError::NotFound) => {}
                Err(e) => return Err(e.into()),
            }
            flat
        }
    };

    let workspace_id = to_write.workspace.id;
    state.store.import_workspace(&to_write).await?;
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "workspace.import".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: serde_json::json!({ "mode": q.mode, "force": q.force }),
        },
    )
    .await;

    Ok(Json(crate::dto::ImportResult {
        workspace_id: workspace_id.0,
        mode: q.mode,
    }))
}

/// Live per-workspace usage counts for metering / quota visibility (Cluster
/// 188). Low-cardinality (a per-request DB aggregate, not a scraped per-tenant
/// series). `workspace:read` — aggregate counts, not content.
pub async fn get_workspace_usage(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<WorkspaceUsage>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    Ok(Json(state.store.workspace_usage(workspace_id).await?))
}

pub async fn replay_quarantined_outbox(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, outbox_id)): Path<(uuid::Uuid, i64)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let backend = state.outbox_backend.as_ref().ok_or_else(|| {
        ApiError::BadRequest("outbox relay is not enabled for this deployment".into())
    })?;
    backend.replay_quarantined(outbox_id, workspace_id).await?;
    let actor_id = if auth.bypass {
        None
    } else {
        Some(auth.member_id)
    };
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id,
            action: "outbox.replay".into(),
            target_kind: Some("outbox".into()),
            target_id: None,
            metadata: serde_json::json!({
                "outbox_id": outbox_id,
                "workspace_id": workspace_id.0,
            }),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Deserialize)]
pub struct QuarantinedOutboxQuery {
    #[serde(default = "default_outbox_list_limit")]
    pub limit: i64,
}

fn default_outbox_list_limit() -> i64 {
    50
}

pub async fn list_quarantined_outbox(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
    Query(q): Query<QuarantinedOutboxQuery>,
) -> ApiResult<Json<Vec<maidan_store::QuarantinedOutboxRow>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let backend = state.outbox_backend.as_ref().ok_or_else(|| {
        ApiError::BadRequest("outbox relay is not enabled for this deployment".into())
    })?;
    let limit = q.limit.clamp(1, 500);
    let rows = backend
        .list_quarantined_for_workspace(workspace_id, limit)
        .await?;
    Ok(Json(rows))
}

pub async fn get_workspace_context(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
    Query(q): Query<WorkspaceContextQuery>,
) -> ApiResult<Json<crate::thread_context::WorkspaceContext>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let limits = crate::thread_context::ThreadContextLimits {
        message_limit: if q.message_limit > 0 {
            q.message_limit
        } else {
            100
        },
        transition_limit: if q.transition_limit > 0 {
            q.transition_limit
        } else {
            50
        },
        message_cursor: None,
        include_edits: q.include_edits,
    };
    let mut packed = crate::thread_context::build_workspace_context(
        state.store.as_ref(),
        workspace_id,
        q.thread_limit.clamp(1, 50),
        q.thread_cursor.map(ThreadId),
        limits,
    )
    .await?;
    // Drop packed threads in private channels the caller can't access
    // (Cluster 160). Cache the per-channel decision.
    if !auth.bypass {
        // Thread-keyed + DM-participant-aware (Cluster 180; channel-keyed leaked
        // DM threads into the workspace-context pack).
        let mut decision: std::collections::HashMap<ThreadId, bool> =
            std::collections::HashMap::new();
        let mut visible = Vec::with_capacity(packed.threads.len());
        for tc in packed.threads {
            let ok = match decision.get(&tc.thread.id) {
                Some(v) => *v,
                None => {
                    let v =
                        maidan_auth::can_access_thread(state.store.as_ref(), &auth, tc.thread.id)
                            .await?;
                    decision.insert(tc.thread.id, v);
                    v
                }
            };
            if ok {
                visible.push(tc);
            }
        }
        packed.threads = visible;
    }
    Ok(Json(packed))
}

pub async fn purge_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<WorkspacePurgeResult>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let mut result = state.store.purge_workspace_messages(workspace_id).await?;
    let mut artifact_blobs_deleted = 0u64;
    for sha_hex in &result.artifact_shas {
        let Ok(sha) = maidan_artifacts::Sha256::from_hex(sha_hex) else {
            continue;
        };
        if state.artifacts.delete(&sha).await.is_ok() {
            artifact_blobs_deleted += 1;
        }
    }
    result.artifact_shas.clear();
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "workspace.purge".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: serde_json::json!({
                "messages_tombstoned": result.messages_tombstoned,
                "messages_purged": result.messages_purged,
                "embeddings_removed": result.embeddings_removed,
                "references_removed": result.references_removed,
                "api_tokens_revoked": result.api_tokens_revoked,
                "events_removed": result.events_removed,
                "artifacts_removed": result.artifacts_removed,
                "artifact_blobs_deleted": artifact_blobs_deleted,
            }),
        })
        .await?;
    let uris = maidan_mcp::resource_updates::uris_for_workspace_purge(workspace_id);
    state.mcp.publish_resource_uris(uris).await;
    Ok(Json(result))
}

pub async fn erase_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<crate::dto::EraseWorkspace>,
) -> ApiResult<Json<WorkspaceEraseResult>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    if body.confirm_workspace_id != workspace_id.0 {
        return Err(ApiError::BadRequest(
            "confirm_workspace_id must match path workspace id".into(),
        ));
    }
    state.store.get_workspace(workspace_id).await?;
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "workspace.erase".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: serde_json::json!({ "phase": "started" }),
        })
        .await?;
    let mut result = state.store.erase_workspace(workspace_id).await?;
    let mut artifact_blobs_deleted = 0u64;
    for sha_hex in &result.purge.artifact_shas {
        let Ok(sha) = maidan_artifacts::Sha256::from_hex(sha_hex) else {
            continue;
        };
        if state.artifacts.delete(&sha).await.is_ok() {
            artifact_blobs_deleted += 1;
        }
    }
    let _ = artifact_blobs_deleted;
    result.purge.artifact_shas.clear();
    let uris = maidan_mcp::resource_updates::uris_for_workspace_purge(workspace_id);
    state.mcp.publish_resource_uris(uris).await;
    Ok(Json(result))
}

pub async fn list_workspace_audit(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<ListAuditQuery>,
) -> ApiResult<Json<Vec<AuditEvent>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let limit = q.limit.clamp(1, 500);
    Ok(Json(
        state
            .store
            .list_audit_for_workspace(workspace_id, limit)
            .await?,
    ))
}

/// `GET /operator/audit` — audit events across **all** workspaces (Cluster 132).
/// Gated by the global `audit:read-global` capability; intentionally **not**
/// `ensure_workspace`-scoped (it spans workspaces).
pub async fn list_global_audit(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<ListAuditQuery>,
) -> ApiResult<Json<Vec<AuditEvent>>> {
    cap(&auth, AUDIT_READ_GLOBAL)?;
    let limit = q.limit.clamp(1, 500);
    Ok(Json(state.store.list_audit(limit).await?))
}

pub async fn list_events(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<ListEventsQuery>,
    auth: Option<Extension<AuthContext>>,
    peer: Option<Extension<PeerContext>>,
) -> ApiResult<Json<Vec<StoredEvent>>> {
    let workspace_id = WorkspaceId(workspace_id);
    match (&auth, &peer) {
        (Some(Extension(auth)), None) => {
            cap(auth, WORKSPACE_READ)?;
            ensure_workspace(auth, workspace_id)?;
        }
        (None, Some(Extension(PeerContext(peer)))) => {
            if peer.remote_workspace_id != workspace_id {
                return Err(ApiError::Forbidden(
                    "peer may only read its registered remote workspace".into(),
                ));
            }
        }
        _ => return Err(ApiError::Unauthorized),
    }
    Ok(Json(
        state
            .store
            .list_events_after(workspace_id, q.after_id, q.limit)
            .await?,
    ))
}
