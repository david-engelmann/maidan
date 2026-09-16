//! Land-gate pointer (Cluster 385.3, renamed Cluster 389). The room
//! holds `{kind:"land_gate", status:pass|fail, artifact_sha?}` plus the
//! green/amber/red land vocabulary. An external verifier records
//! pass/fail. Writes are `thread:transition`; reads are `workspace:read`.
//! The FSM close-gate (385.2) enforces a qualifying green pass.
//!
//! **A gate ratchets** (Cluster 397.2). Arming or recording against it is
//! `thread:transition`, but *removing* it is `channel:admin`. Clearing the row
//! makes `gate_in_tx` vacuous, so a clear is exactly as powerful as a close —
//! and `thread:transition` is the capability a close already needs, and is in
//! the `maidan.agent.worker` bundle. Guarding both with it let the constrained
//! agent delete its own constraint in one extra call, and left no trace.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{CHANNEL_ADMIN, THREAD_TRANSITION, WORKSPACE_READ},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ApiResult};
use crate::dto::SetLandGate;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn set_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetLandGate>,
) -> ApiResult<Json<LandGateStanding>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    let sha = body
        .artifact_sha
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let standing = state
        .store
        .set_land_gate_pointer(thread_id, auth.member_id, body.status, sha, body.land)
        .await?;
    Ok(Json(standing))
}

pub async fn get_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<LandGateStanding>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.get_land_gate_standing(thread_id).await?))
}

/// Remove the gate entirely — `channel:admin`, and audited. This is the waiver,
/// not a write against the gate, so it answers to the administrative capability
/// rather than the one the gate constrains.
pub async fn clear_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, CHANNEL_ADMIN)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    if state.store.clear_land_gate(thread_id).await? {
        crate::audit::record(
            &state,
            NewAuditEvent {
                actor_id: Some(auth.member_id),
                action: "land_gate.clear".into(),
                target_kind: Some("thread".into()),
                target_id: Some(thread_id.0),
                metadata: serde_json::json!({}),
            },
        )
        .await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn require_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<LandGateStanding>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.require_land_gate(thread_id).await?))
}
