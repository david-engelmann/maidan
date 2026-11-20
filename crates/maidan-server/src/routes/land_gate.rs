//! Land-gate pointer. The room holds `{kind:"land_gate", status:pass|fail,
//! artifact_sha?}` plus the green/amber/red land vocabulary. An external
//! verifier records pass/fail. Writes are `thread:transition`; reads are
//! `workspace:read`. The FSM close-gate (385.2) enforces a qualifying green
//! pass.
//!
//! **A gate ratchets**. Arming or recording against it is
//! `thread:transition`, but *removing* it is `channel:admin`. Clearing the row
//! makes `gate_in_tx` vacuous, so a clear is exactly as powerful as a close —
//! and `thread:transition` is the capability a close already needs, and is in
//! the `maidan.agent.worker` bundle. Guarding both with it let the constrained
//! agent delete its own constraint in one extra call, and left no trace.

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::{
    capability::{CHANNEL_ADMIN, THREAD_TRANSITION, TOKEN_ADMIN, WORKSPACE_READ},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::{SetLandGate, SetLandGateAdvisor};
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::land_gate_advisor::{
    LandGateAdvice, LandGateAdviceRequest, LandGateAdvisorError, LandGateAdvisorStatus,
    WORKSPACE_KEY_SECRET,
};
use crate::session::SessionContext;
use crate::state::AppState;

/// Ask the decision model for advice on a hand-off. This route never writes
/// the pointer: a qualified external verifier remains the only authority that
/// can call [`set_land_gate`]. The packet the model reads is built from the
/// thread, so a caller cannot choose what leaves.
pub async fn advise_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<LandGateAdviceRequest>,
) -> ApiResult<Json<LandGateAdvice>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    let advisor = state.land_gate_advisor.as_ref().ok_or(ApiError::NotFound)?;
    advisor
        .advise(
            state.store.as_ref(),
            state.federation.encryption_key.as_deref(),
            auth.workspace_id(),
            thread_id,
            body,
        )
        .await
        .map(Json)
        .map_err(|error| advisor_error(error, "land-gate advice"))
}

/// One answer for every advisor failure: the caller's mistake is a 400, a
/// thread or workspace that is not ready a 409, and the provider's trouble a
/// 502 that says the gate itself is untouched.
fn advisor_error(error: LandGateAdvisorError, what: &str) -> ApiError {
    match error {
        LandGateAdvisorError::InvalidRequest(error) => ApiError::BadRequest(error),
        LandGateAdvisorError::NotReady(error) => ApiError::Conflict(error),
        LandGateAdvisorError::Store(error) => error.into(),
        error => {
            tracing::warn!(%error, what, "land-gate advisor request failed");
            ApiError::BadGateway(
                "land-gate advisor unavailable; the authoritative gate is unchanged".into(),
            )
        }
    }
}

/// Who may set or forget a workspace's advisor key: the workspace's admin.
/// A bearer needs `token:admin`, as for the delivery allowlist: the key is
/// the workspace's choice of where its hand-offs go. In the console it is a
/// person signed in who holds `token:admin` on a live token, or may mint the
/// first one, so the session lends them nothing a token of theirs could not
/// do. A session made from a token is refused, as on the console's other
/// admin panels: that token's holder uses the bearer route.
async fn advisor_admin(
    state: &AppState,
    auth: &AuthContext,
    session: Option<&SessionContext>,
    workspace_id: WorkspaceId,
) -> ApiResult<()> {
    let Some(session) = session else {
        cap(auth, TOKEN_ADMIN)?;
        return ensure_workspace(auth, workspace_id);
    };
    ensure_workspace(auth, workspace_id)?;
    if session.token.is_some() {
        return Err(ApiError::Forbidden(
            "a session made from a token cannot set the advisor key here; that token's holder uses PUT /workspaces/{id}/land-gate/advisor".into(),
        ));
    }
    let member = state
        .store
        .get_member_in(workspace_id, session.member_id)
        .await?;
    if member.kind == MemberKind::Human
        && super::member::session_holds_or_may_mint(state, session, TOKEN_ADMIN).await?
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "only the workspace's admin can set the advisor key".into(),
        ))
    }
}

/// `GET /workspaces/:id/land-gate/advisor`: whether this instance offers
/// advice, and whose key a call from this workspace would use. Never the key.
pub async fn get_land_gate_advisor(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<LandGateAdvisorStatus>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let Some(advisor) = state.land_gate_advisor.as_ref() else {
        return Ok(Json(LandGateAdvisorStatus::disabled()));
    };
    advisor
        .status(state.store.as_ref(), workspace_id)
        .await
        .map(Json)
        .map_err(|error| advisor_error(error, "advisor status"))
}

/// `PUT /workspaces/:id/land-gate/advisor`: store this workspace's own
/// TypeSafe key, after the provider accepts it. The key is kept as a
/// workspace secret under a name the server keeps for itself, encrypted at
/// rest, out of reach of the generic secret routes, and is sent only to the
/// provider's host.
pub async fn set_land_gate_advisor(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<SetLandGateAdvisor>,
) -> ApiResult<Json<LandGateAdvisorStatus>> {
    let workspace_id = WorkspaceId(id);
    advisor_admin(&state, &auth, session.as_ref().map(|s| &s.0), workspace_id).await?;
    let advisor = state.land_gate_advisor.as_ref().ok_or(ApiError::NotFound)?;
    let api_key = body.api_key.trim();
    if api_key.is_empty() || api_key.len() > 512 || api_key.chars().any(char::is_whitespace) {
        return Err(ApiError::BadRequest(
            "api_key is 1 to 512 characters with no spaces".into(),
        ));
    }
    // Not a failure of this request: the instance was set up without the key
    // that encrypts workspace secrets, so it has nowhere to keep one.
    let key = state.federation.encryption_key.as_deref().ok_or_else(|| {
        ApiError::Conflict(
            "this instance cannot store a workspace's key (it has no FEDERATION_ENCRYPTION_KEY); advice here uses the instance's key"
                .into(),
        )
    })?;
    advisor
        .check_key(api_key)
        .await
        .map_err(|error| advisor_error(error, "advisor key check"))?;
    let value_ciphertext = maidan_auth::encrypt_peer_secret(api_key, key)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let actor = auth.actor_id();
    state
        .store
        .create_secret_audited(
            NewSecret {
                workspace_id,
                name: WORKSPACE_KEY_SECRET.into(),
                value_ciphertext,
                created_by: auth.member_id(),
            },
            Box::new(move |secret| NewAuditEvent {
                scope: AuditScope::Workspace(secret.workspace_id),
                actor_id: Some(actor),
                action: "land_gate_advisor.set_key".into(),
                target_kind: Some("secret".into()),
                target_id: Some(secret.id.0),
                metadata: serde_json::json!({
                    "workspace_id": secret.workspace_id.0,
                    "name": secret.name,
                }),
            }),
        )
        .await?;
    // What was just committed, not a second read that could fail after it.
    Ok(Json(advisor.status_with_own_key()))
}

/// `DELETE /workspaces/:id/land-gate/advisor`: forget this workspace's key.
/// Advice then uses the instance's key, or is refused when it has none.
pub async fn clear_land_gate_advisor(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<LandGateAdvisorStatus>> {
    let workspace_id = WorkspaceId(id);
    advisor_admin(&state, &auth, session.as_ref().map(|s| &s.0), workspace_id).await?;
    let advisor = state.land_gate_advisor.as_ref().ok_or(ApiError::NotFound)?;
    state
        .store
        .delete_secret_audited(
            workspace_id,
            WORKSPACE_KEY_SECRET,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id()),
                action: "land_gate_advisor.clear_key".into(),
                target_kind: Some("secret".into()),
                target_id: None,
                metadata: serde_json::json!({
                    "workspace_id": workspace_id.0,
                    "name": WORKSPACE_KEY_SECRET,
                }),
            },
        )
        .await?;
    Ok(Json(advisor.status_without_own_key()))
}

pub async fn set_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
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
        .set_land_gate_pointer(thread_id, auth.member_id(), body.status, sha, body.land)
        .await?;
    Ok(Json(standing))
}

pub async fn get_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<LandGateStanding>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.get_land_gate_standing(thread_id).await?))
}

/// Every land-gate verdict recorded on a thread, oldest first. A new pointer
/// or clearing the gate leaves earlier verdicts in place.
pub async fn list_land_gate_history(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<LandGateVerdict>>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.list_land_gate_history(thread_id).await?))
}

/// Remove the gate entirely — `channel:admin`, and audited. This is the waiver,
/// not a write against the gate, so it answers to the administrative capability
/// rather than the one the gate constrains.
pub async fn clear_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, CHANNEL_ADMIN)?;
    let thread_id = ThreadId(id);
    let scope = maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    let cleared = state
        .store
        .clear_land_gate_audited(
            thread_id,
            NewAuditEvent {
                scope: AuditScope::Workspace(scope.workspace_id),
                actor_id: Some(auth.actor_id()),
                action: "land_gate.clear".into(),
                target_kind: Some("thread".into()),
                target_id: Some(thread_id.0),
                metadata: serde_json::json!({}),
            },
        )
        .await?;
    if cleared {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn require_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<LandGateStanding>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.require_land_gate(thread_id).await?))
}
