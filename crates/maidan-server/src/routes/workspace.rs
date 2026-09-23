//! Workspace lifecycle handlers: create/get, context, purge/erase, audit,
//! events, and the quarantined-outbox replay/list endpoints.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{
        AUDIT_READ_GLOBAL, OPERATOR_GLOBAL, TOKEN_ADMIN, WORKSPACE_READ, WORKSPACE_WRITE,
    },
    AuthContext,
};
use maidan_store::StoreError;
use maidan_types::*;

#[cfg(feature = "bootstrap")]
use super::publish_stored;
use super::{cap, clamp_context_transition_limit, ensure_workspace, ApiResult};
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

/// Export the whole workspace content graph as a signed
/// `maidan.workspace.export/1` envelope. Gated on `token:admin`. Tokens die on
/// export — secrets are omitted and the envelope records `token_policy`.
/// Refuses if the operator signing key is not configured.
pub async fn export_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<SignedExport>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let bundle = crate::export::build(&state.store, workspace_id).await?;
    let signed = crate::export::sign_bundle(state.export_signing.as_ref(), &bundle)?;
    // A read, but the whole workspace leaves in it — who took it belongs in the
    // record as much as any change does.
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.actor_id),
            action: "workspace.export".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: serde_json::json!({ "content_sha256": signed.content_sha256 }),
        },
    )
    .await;
    Ok(Json(signed))
}

/// Verify a signed export without importing it. A blank instance uses this
/// to check the file before `POST /workspaces/import`. `token:admin`.
pub async fn verify_workspace_export(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(envelope): ApiJson<SignedExport>,
) -> ApiResult<Json<crate::dto::VerifyExportResult>> {
    cap(&auth, TOKEN_ADMIN)?;
    crate::export::verify_bundle(&envelope, &state.export_verify_keys)?;
    Ok(Json(crate::dto::VerifyExportResult {
        ok: true,
        token_policy: envelope.token_policy,
        public_key: envelope.public_key,
        content_sha256: envelope.content_sha256,
        workspace_id: crate::export::payload_workspace_id(&envelope.payload),
    }))
}

/// Operator public key for out-of-band authenticity pins. `token:admin`.
pub async fn export_public_key(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<crate::dto::ExportPublicKey>> {
    cap(&auth, TOKEN_ADMIN)?;
    let key = state.export_signing.as_ref().ok_or_else(|| {
        ApiError::BadRequest(
            "export signing key is not configured (MAIDAN_EXPORT_SIGNING_KEY)".into(),
        )
    })?;
    Ok(Json(crate::dto::ExportPublicKey {
        alg: SIGNED_EXPORT_ALG.into(),
        public_key: key.public_key_hex(),
        token_policy: TokenPolicy::TokensDieOnExport,
    }))
}

/// Import a **signed** workspace bundle. Verification is fail-closed (tamper /
/// bad sig / secret fields / wrong pin). Gated on `token:admin`. `mode=new`
/// remaps ids; `mode=restore` preserves them (409 if that workspace exists
/// unless `force=true`).
///
/// **The signature authorizes nothing**. It proves the bundle
/// is internally consistent with its own embedded key — and with the documented
/// no-pin default (`MAIDAN_EXPORT_VERIFY_KEYS` unset) that key can be the
/// caller's. So the workspace a `restore` names is a caller-supplied id and is
/// scoped like every other one: `token:admin` is per-workspace, and a restore
/// that names someone else's workspace is `403`, not a signature question.
/// Without that check an admin could sign a bundle naming any tenant and have
/// `force=true` erase it.
pub async fn import_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<crate::dto::ImportQuery>,
    ApiJson(envelope): ApiJson<SignedExport>,
) -> ApiResult<Json<crate::dto::ImportResult>> {
    use crate::dto::ImportMode;
    cap(&auth, TOKEN_ADMIN)?;
    crate::export::verify_bundle(&envelope, &state.export_verify_keys)?;
    let bundle = crate::export::inner_bundle(&envelope)?;

    let flat = crate::import::flatten(bundle);
    let to_write = match q.mode {
        ImportMode::New => crate::import::remap(flat, uuid::Uuid::new_v4),
        ImportMode::Restore => {
            // A restore writes to the id inside the bundle, so that id is the
            // authorization subject — not the token's own workspace by assumption.
            ensure_workspace(&auth, flat.workspace.id)?;
            let existing = state.store.get_workspace(flat.workspace.id).await;
            match existing {
                Ok(_) if !q.force => {
                    return Err(ApiError::Conflict(format!(
                        "workspace {} already exists; retry with force=true to overwrite",
                        flat.workspace.id.0
                    )));
                }
                Ok(_) => {
                    // force: erase the existing workspace so the restore lands
                    // cleanly. This is the same destruction `erase_workspace`
                    // performs, so it answers to the same guards — a legal hold
                    // refuses it, and the intent is audited *before* the rows go.
                    ensure_not_under_legal_hold(&state, flat.workspace.id).await?;
                    state
                        .store
                        .append_audit(NewAuditEvent {
                            actor_id: Some(auth.actor_id),
                            action: "workspace.import".into(),
                            target_kind: Some("workspace".into()),
                            target_id: Some(flat.workspace.id.0),
                            metadata: serde_json::json!({
                                "phase": "erase_started",
                                "mode": q.mode,
                                "force": q.force,
                            }),
                        })
                        .await?;
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
            actor_id: Some(auth.actor_id),
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

/// Live per-workspace usage counts for metering / quota visibility.
/// Low-cardinality (a per-request DB aggregate, not a scraped per-tenant
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

/// Tombstone / deletion explorer. Soft-deleted messages in this workspace (body
/// already cleared); `include_purged` reconstructs hard-deleted rows from
/// `MessageTombstoned`. `workspace:read`; inaccessible private-channel / DM
/// rows are dropped.
pub async fn list_workspace_tombstones(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListTombstonesQuery>,
) -> ApiResult<Json<Vec<TombstoneRecord>>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    if let Some(cid) = q.channel_id {
        maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, ChannelId(cid)).await?;
    }
    if let Some(tid) = q.thread_id {
        maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, ThreadId(tid)).await?;
    }
    let limit = maidan_types::clamp_tombstone_limit(q.limit);
    let rows = state
        .store
        .list_tombstones(
            workspace_id,
            q.channel_id.map(ChannelId),
            q.thread_id.map(ThreadId),
            q.include_purged,
            limit,
        )
        .await?;
    if auth.bypass {
        return Ok(Json(rows));
    }
    let mut visible = Vec::with_capacity(rows.len());
    for row in rows {
        if maidan_auth::can_access_thread(state.store.as_ref(), &auth, row.thread_id).await? {
            visible.push(row);
        }
    }
    Ok(Json(visible))
}

/// EventKind census for a workspace. Optional channel/thread narrowing.
/// `workspace:read`; private channels the caller cannot access are excluded in
/// the query (`private_channel_deny_set`).
pub async fn get_workspace_kind_census(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<KindCensusQuery>,
) -> ApiResult<Json<KindCensus>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    if let Some(cid) = q.channel_id {
        maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, ChannelId(cid)).await?;
    }
    if let Some(tid) = q.thread_id {
        maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, ThreadId(tid)).await?;
    }
    let deny =
        maidan_auth::private_channel_deny_set(state.store.as_ref(), &auth, workspace_id).await?;
    Ok(Json(
        state
            .store
            .event_kind_census(
                workspace_id,
                q.channel_id.map(ChannelId),
                q.thread_id.map(ThreadId),
                &deny,
            )
            .await?,
    ))
}

/// Workspace-scoped thread-result list. Optional exact-match `result_kind`
/// facet on the namespaced string (e.g. `example.review.result/1`).
/// `workspace:read`; private-channel rows the caller cannot access are dropped.
pub async fn list_workspace_results(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListThreadResultsQuery>,
) -> ApiResult<Json<Vec<ThreadResult>>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let results = state
        .store
        .list_thread_results(workspace_id, q.result_kind.as_deref(), limit)
        .await?;
    if auth.bypass {
        return Ok(Json(results));
    }
    let mut visible = Vec::with_capacity(results.len());
    for result in results {
        if maidan_auth::can_access_thread(state.store.as_ref(), &auth, result.thread_id).await? {
            visible.push(result);
        }
    }
    Ok(Json(visible))
}

/// Threads in this workspace that share a producer `parent_run_id`.
/// `workspace:read`; private-channel rows the caller cannot access are dropped.
/// F7 mute is not consulted.
pub async fn list_run_threads(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<RunLineageQuery>,
) -> ApiResult<Json<Vec<Thread>>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let parent_run_id = normalize_parent_run_id(&q.parent_run_id).ok_or_else(|| {
        ApiError::BadRequest(
            "parent_run_id must be a non-empty producer run id (max 256 bytes)".into(),
        )
    })?;
    let threads = state
        .store
        .list_threads_for_run(workspace_id, parent_run_id)
        .await?;
    if auth.bypass {
        return Ok(Json(threads));
    }
    let mut visible = Vec::with_capacity(threads.len());
    for thread in threads {
        if maidan_auth::can_access_thread(state.store.as_ref(), &auth, thread.id).await? {
            visible.push(thread);
        }
    }
    Ok(Json(visible))
}

/// Nested occupancy for a producer run: queued / claimed / working / blocked
/// across every **open** thread that shares `parent_run_id`. `workspace:read`.
/// F7 mute stays orthogonal (a muted nested thread still counts). Empty /
/// unknown run → zeros, not 404.
pub async fn get_run_occupancy(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<RunLineageQuery>,
) -> ApiResult<Json<RunOccupancy>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let parent_run_id = normalize_parent_run_id(&q.parent_run_id).ok_or_else(|| {
        ApiError::BadRequest(
            "parent_run_id must be a non-empty producer run id (max 256 bytes)".into(),
        )
    })?;
    Ok(Json(
        state
            .store
            .run_occupancy(workspace_id, parent_run_id)
            .await?,
    ))
}

/// `PUT /workspaces/:wid/wip-limit` — set or clear the workspace's WIP limit
/// (max concurrent live claims per member). `{limit: n}` caps (0 freezes);
/// `{limit: null}` removes the cap. `workspace:write`.
pub async fn set_wip_limit(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetWipLimit>,
) -> ApiResult<Json<WipLimitView>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    if let Some(limit) = body.limit {
        if limit < 0 {
            return Err(ApiError::BadRequest("wip limit must be >= 0".into()));
        }
    }
    state.store.set_wip_limit(workspace_id, body.limit).await?;
    Ok(Json(WipLimitView { limit: body.limit }))
}

/// `GET /workspaces/:wid/wip-limit` — the workspace's WIP limit, or `null` when
/// unset (unlimited). `workspace:read`.
pub async fn get_wip_limit(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<WipLimitView>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(WipLimitView {
        limit: state.store.get_wip_limit(workspace_id).await?,
    }))
}

/// `PUT /workspaces/:wid/delegation-policy` — set the longest a delegation grant
/// may live, in days (1–3650), or `null` for the default 90. It bounds the
/// standing authority to mint delegated tokens, so it is `token:admin` — the
/// capability that issues grants — not `workspace:write`. Applies to grants
/// issued afterwards; revoke an existing grant to end it sooner.
pub async fn set_delegation_policy(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<crate::dto::SetDelegationPolicy>,
) -> ApiResult<Json<maidan_types::DelegationPolicy>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let policy = state
        .store
        .set_delegation_policy(workspace_id, body.max_grant_days)
        .await?;
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.actor_id),
            action: "delegation_policy.set".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: serde_json::json!({
                "max_grant_days": policy.max_grant_days,
                "is_default": policy.is_default,
            }),
        },
    )
    .await;
    Ok(Json(policy))
}

/// `GET /workspaces/:wid/delegation-policy` — the grant ceiling in force.
/// `workspace:read`: anyone who can be granted authority may see its bound.
pub async fn get_delegation_policy(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<maidan_types::DelegationPolicy>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.get_delegation_policy(workspace_id).await?))
}

/// `PUT /workspaces/:id/spawn-budget` — set the workspace's spawn budget: max
/// direct child threads per parent, max thread nesting depth, max tool calls
/// per thread. A full replace — an omitted or `null` axis is unlimited, so `{}`
/// clears the budget; `0` freezes an axis. Enforced on thread create and
/// message post (376.3). Set the caps well below the fan-out a hosted agent
/// platform allows: coordination cost grows as n(n−1)/2. `workspace:write`.
pub async fn set_spawn_budget(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetSpawnBudget>,
) -> ApiResult<Json<SpawnBudgetView>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    for (axis, limit) in [
        ("max_children", body.max_children),
        ("max_depth", body.max_depth),
        ("max_tools", body.max_tools),
    ] {
        if limit.is_some_and(|l| l < 0) {
            return Err(ApiError::BadRequest(format!("{axis} must be >= 0")));
        }
    }
    state
        .store
        .set_spawn_budget(
            workspace_id,
            body.max_children,
            body.max_depth,
            body.max_tools,
        )
        .await?;
    Ok(Json(SpawnBudgetView {
        max_children: body.max_children,
        max_depth: body.max_depth,
        max_tools: body.max_tools,
    }))
}

/// `GET /workspaces/:id/spawn-budget` — the workspace's spawn budget; every
/// axis is `null` when unset (unlimited). `workspace:read`.
pub async fn get_spawn_budget(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<SpawnBudgetView>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let budget = state.store.get_spawn_budget(workspace_id).await?;
    Ok(Json(SpawnBudgetView {
        max_children: budget.as_ref().and_then(|b| b.max_children),
        max_depth: budget.as_ref().and_then(|b| b.max_depth),
        max_tools: budget.as_ref().and_then(|b| b.max_tools),
    }))
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
        transition_limit: clamp_context_transition_limit(q.transition_limit),
        message_cursor: None,
        include_edits: q.include_edits,
        include_glossary: q.include_glossary,
        as_of: None, // as-of replay is thread-scoped
        token_budget: q.token_budget,
        // Overridden to false per nested thread inside build_workspace_context
        // (grounding / accepted decisions are the focused single-thread view).
        include_parent_grounding: false,
        include_accepted_decisions: false,
    };
    let mut packed = crate::thread_context::build_workspace_context(
        state.store.as_ref(),
        workspace_id,
        q.thread_limit.clamp(1, 50),
        q.thread_cursor.map(ThreadId),
        limits,
    )
    .await?;
    // Drop packed threads in private channels the caller can't access. Cache
    // the per-channel decision.
    if !auth.bypass {
        // Thread-keyed + DM-participant-aware.
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
    // Destroying a workspace's content is authority over the workspace, not work
    // in it. It was `workspace:write`, which every agent holds and delegation
    // lends — while placing a legal hold to *protect* the same data required
    // `token:admin`, and so did exporting it. Destruction is now at least as
    // privileged as copying or preserving.
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    ensure_not_under_legal_hold(&state, workspace_id).await?;
    let mut result = state.store.purge_workspace_messages(workspace_id).await?;
    let artifact_blobs_deleted = delete_orphaned_blobs(&state, &result.artifact_shas).await;
    result.artifact_shas.clear();
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.actor_id),
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

/// Delete the blobs a purge orphaned — shas no workspace references any more.
///
/// The store decides orphanhood inside its transaction; this runs after it, so
/// a workspace that uploaded the same bytes in between must not lose them. A
/// blob whose artifact row exists again is kept.
async fn delete_orphaned_blobs(state: &AppState, shas: &[String]) -> u64 {
    let mut deleted = 0u64;
    for sha_hex in shas {
        let Ok(sha) = maidan_artifacts::Sha256::from_hex(sha_hex) else {
            continue;
        };
        if state.store.get_artifact_by_sha(sha_hex).await.is_ok() {
            continue;
        }
        if state.artifacts.delete(&sha).await.is_ok() {
            deleted += 1;
        }
    }
    deleted
}

pub async fn erase_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<crate::dto::EraseWorkspace>,
) -> ApiResult<Json<WorkspaceEraseResult>> {
    let workspace_id = WorkspaceId(id);
    // See `purge_workspace`. The confirmation below guards against a mistake,
    // not an adversary: it must equal the id already in the URL.
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    if body.confirm_workspace_id != workspace_id.0 {
        return Err(ApiError::BadRequest(
            "confirm_workspace_id must match path workspace id".into(),
        ));
    }
    state.store.get_workspace(workspace_id).await?;
    ensure_not_under_legal_hold(&state, workspace_id).await?;
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.actor_id),
            action: "workspace.erase".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: serde_json::json!({ "phase": "started" }),
        })
        .await?;
    let mut result = state.store.erase_workspace(workspace_id).await?;
    let artifact_blobs_deleted = delete_orphaned_blobs(&state, &result.purge.artifact_shas).await;
    let _ = artifact_blobs_deleted;
    result.purge.artifact_shas.clear();
    let uris = maidan_mcp::resource_updates::uris_for_workspace_purge(workspace_id);
    state.mcp.publish_resource_uris(uris).await;
    Ok(Json(result))
}

/// Refuse a destructive workspace operation while the workspace is under a
/// legal hold — 409 Conflict. Read on the primary (the pg store routes
/// `get_legal_hold` there) so a lagged replica can never let evidence be
/// destroyed.
async fn ensure_not_under_legal_hold(state: &AppState, workspace_id: WorkspaceId) -> ApiResult<()> {
    if state.store.get_legal_hold(workspace_id).await?.is_some() {
        return Err(ApiError::Conflict(
            "workspace is under a legal hold; lift it before deleting workspace data".into(),
        ));
    }
    Ok(())
}

/// `PUT /workspaces/:id/legal-hold` — place (or update) a legal hold.
/// `token:admin` (a higher bar than the `workspace:write` that purges, so a
/// workspace admin can't lift-then-destroy). Body `{reason}`.
pub async fn place_legal_hold(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<PlaceLegalHold>,
) -> ApiResult<Json<LegalHold>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let reason = body.reason.trim();
    if reason.is_empty() {
        return Err(ApiError::BadRequest("reason must not be empty".into()));
    }
    state.store.get_workspace(workspace_id).await?;
    let hold = state
        .store
        .place_legal_hold(workspace_id, reason, Some(auth.member_id))
        .await?;
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.actor_id),
            action: "legal_hold.place".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: serde_json::json!({ "reason": reason }),
        },
    )
    .await;
    Ok(Json(hold))
}

/// `DELETE /workspaces/:id/legal-hold` — lift the hold. `204` when a hold
/// existed, `404` when not. `token:admin`.
pub async fn lift_legal_hold(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    if state.store.lift_legal_hold(workspace_id).await? {
        crate::audit::record(
            &state,
            NewAuditEvent {
                actor_id: Some(auth.actor_id),
                action: "legal_hold.lift".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(workspace_id.0),
                metadata: serde_json::json!({}),
            },
        )
        .await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// `GET /workspaces/:id/legal-hold` — the hold, or `404`. `workspace:read`.
pub async fn get_legal_hold(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<LegalHold>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    match state.store.get_legal_hold(workspace_id).await? {
        Some(hold) => Ok(Json(hold)),
        None => Err(ApiError::NotFound),
    }
}

/// `GET /operator/legal-holds` — every active hold across all workspaces,
/// newest first. `token:admin` — the operator/compliance view. `GET
/// /operator/legal-holds` — every workspace's hold.
///
/// `operator:global`, not the per-workspace `token:admin` this used to take:
/// the query is instance-wide and deliberately stays that way, because scoping
/// it to the caller would make it a duplicate of `GET
/// /workspaces/:id/legal-hold`. A genuinely global read needs a genuinely
/// global capability.
pub async fn list_legal_holds(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<Vec<LegalHold>>> {
    cap(&auth, OPERATOR_GLOBAL)?;
    Ok(Json(state.store.list_legal_holds().await?))
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

/// `GET /operator/audit` — audit events across **all** workspaces. Gated by the
/// global `audit:read-global` capability; intentionally **not**
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

fn ensure_event_log_read(
    auth: &Option<Extension<AuthContext>>,
    peer: &Option<Extension<PeerContext>>,
    workspace_id: WorkspaceId,
) -> ApiResult<()> {
    match (auth, peer) {
        (Some(Extension(auth)), None) => {
            cap(auth, WORKSPACE_READ)?;
            ensure_workspace(auth, workspace_id)?;
            Ok(())
        }
        (None, Some(Extension(PeerContext(peer)))) => {
            if peer.remote_workspace_id != workspace_id {
                Err(ApiError::Forbidden(
                    "peer may only read its registered remote workspace".into(),
                ))
            } else {
                Ok(())
            }
        }
        _ => Err(ApiError::Unauthorized),
    }
}

pub async fn list_events(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<ListEventsQuery>,
    auth: Option<Extension<AuthContext>>,
    peer: Option<Extension<PeerContext>>,
) -> ApiResult<Json<Vec<StoredEvent>>> {
    let workspace_id = WorkspaceId(workspace_id);
    ensure_event_log_read(&auth, &peer, workspace_id)?;
    if q.after_id < 0 {
        return Err(ApiError::BadRequest("after_id must be non-negative".into()));
    }
    let types = match q.types.as_deref() {
        Some(s) => parse_projector_types(s).map_err(ApiError::BadRequest)?,
        None => Vec::new(),
    };
    let shape = ProjectorShape {
        workspace_id,
        channel_id: q.channel_id.map(ChannelId),
        thread_id: q.thread_id.map(ThreadId),
        types,
    };
    let mut after_id = q.after_id;
    if let Some(ref consumer_id) = q.consumer_id {
        crate::delivery::validate_consumer_id(consumer_id).map_err(ApiError::BadRequest)?;
        after_id = crate::delivery::effective_subscribe_after_id(
            state.store.as_ref(),
            Some(consumer_id.as_str()),
            Some(workspace_id),
            after_id,
        )
        .await
        .map_err(|e| ApiError::from(e).with_snapshot(workspace_id))?;
    }
    Ok(Json(
        crate::delivery::list_events_for_shape(
            state.store.as_ref(),
            &shape,
            after_id,
            q.limit.clamp(1, 500),
        )
        .await
        .map_err(|e| ApiError::from(e).with_snapshot(workspace_id))?,
    ))
}

/// Verify the retained hash chain for this workspace. 200 when intact; 409
/// `event-log-broken` when a splice or rewrite is detected. Same auth as
/// [`list_events`] so a federated peer can check without trusting the host
/// process.
pub async fn verify_event_chain(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    auth: Option<Extension<AuthContext>>,
    peer: Option<Extension<PeerContext>>,
) -> ApiResult<Json<ChainVerifyReport>> {
    let workspace_id = WorkspaceId(workspace_id);
    ensure_event_log_read(&auth, &peer, workspace_id)?;
    let report = state.store.verify_event_chain(workspace_id).await?;
    if !report.ok {
        return Err(ApiError::EventLogBroken {
            break_at: report.break_at,
            reason: report.reason.unwrap_or(ChainBreakReason::MalformedHash),
        });
    }
    Ok(Json(report))
}

/// Hashed domain-graph checkpoint. Header + `graph_hash` is `workspace:read` /
/// federation peer. `include_graph=true` is the full dump — `token:admin` or a
/// registered peer (same bar as export).
pub async fn get_log_snapshot(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<LogSnapshotQuery>,
    auth: Option<Extension<AuthContext>>,
    peer: Option<Extension<PeerContext>>,
) -> ApiResult<Json<LogSnapshot>> {
    let workspace_id = WorkspaceId(workspace_id);
    if q.include_graph {
        match (&auth, &peer) {
            (Some(Extension(auth)), None) => {
                cap(auth, TOKEN_ADMIN)?;
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
    } else {
        ensure_event_log_read(&auth, &peer, workspace_id)?;
    }
    let snap =
        maidan_store::build_log_snapshot(state.store.as_ref(), workspace_id, q.include_graph)
            .await?;
    Ok(Json(snap))
}

/// Since-LSN catch-up page. Same auth as [`list_events`]. A pruned-gap cursor
/// is 409 `must_refetch` with a `snapshot` href; a broken chain is 409
/// `event-log-broken`.
pub async fn catch_up_events(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<CatchUpQuery>,
    auth: Option<Extension<AuthContext>>,
    peer: Option<Extension<PeerContext>>,
) -> ApiResult<Json<CatchUpPage>> {
    let workspace_id = WorkspaceId(workspace_id);
    ensure_event_log_read(&auth, &peer, workspace_id)?;
    let page =
        maidan_store::catch_up_since(state.store.as_ref(), workspace_id, q.after_lsn, q.limit)
            .await
            .map_err(|e| ApiError::from(e).with_snapshot(workspace_id))?;
    if !page.ok() {
        return Err(ApiError::EventLogBroken {
            break_at: page.chain.break_at,
            reason: page.chain.reason.unwrap_or(ChainBreakReason::MalformedHash),
        });
    }
    Ok(Json(page))
}
