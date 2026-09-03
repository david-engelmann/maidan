//! Approval-gate REST surface (Cluster 350.3): the human side of the held gate.
//!
//! A workspace member lists the pending gates (each carrying a server-issued
//! `request_state`) and answers one accept/decline/cancel. The `request_state`
//! is an HMAC over the gate id — the `/ui` is an untrusted client, so the answer
//! must echo a token the server actually issued, verified before the resolve.
//! The resolve is a compare-and-set on `pending` (Cluster 350.1), so a second
//! answer — or a late answer after cancel — is a no-op (silence is not consent).

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use hmac::{Hmac, Mac};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::{ApprovalGate, ApprovalGateId, ApprovalGateState, WorkspaceId};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

/// HMAC-SHA256 over the gate id, hex-encoded — the `request_state` a human
/// echoes back to answer a gate. Mirrors the session-cookie signing (same
/// server secret).
fn sign_request_state(gate_id: ApprovalGateId, secret: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret)
        .unwrap_or_else(|_| unreachable!("HMAC-SHA256 accepts any key length"));
    mac.update(gate_id.0.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time check that `token` is the server's `request_state` for `gate_id`.
fn verify_request_state(token: &str, gate_id: ApprovalGateId, secret: &[u8]) -> bool {
    let expected = sign_request_state(gate_id, secret);
    match (hex::decode(token), hex::decode(&expected)) {
        (Ok(actual), Ok(want)) => actual.len() == want.len() && bool::from(actual.ct_eq(&want)),
        _ => false,
    }
}

/// List a workspace's pending approval gates, each with its `request_state`
/// (Cluster 350.3). `workspace:read`.
pub async fn list_approval_gates(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<ApprovalGateView>>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(wid);
    ensure_workspace(&auth, workspace_id)?;
    let secret = state.subscribe_resume_secret();
    let gates = state
        .store
        .list_pending_approval_gates(workspace_id, 200)
        .await?;
    let views = gates
        .into_iter()
        .map(|gate| {
            let request_state = sign_request_state(gate.id, secret);
            ApprovalGateView {
                gate,
                request_state,
            }
        })
        .collect();
    Ok(Json(views))
}

/// Answer a pending approval gate — accept / decline / cancel (Cluster 350.3).
/// `workspace:write`. The `request_state` is integrity-verified and the gate
/// must be in the caller's workspace. Resolve is a CAS on `pending`, so a
/// second answer is a no-op → `409`.
pub async fn answer_approval_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<AnswerApprovalGate>,
) -> ApiResult<Json<ApprovalGate>> {
    cap(&auth, WORKSPACE_WRITE)?;
    let gate_id = ApprovalGateId(id);
    // Empty accept is not yes; an unknown action is a bad request, never a silent accept.
    let target = match body.action.as_str() {
        "accept" => ApprovalGateState::Accepted,
        "decline" => ApprovalGateState::Declined,
        "cancel" => ApprovalGateState::Cancelled,
        _ => {
            return Err(ApiError::BadRequest(
                "action must be accept, decline, or cancel".into(),
            ))
        }
    };
    // The gate must exist in the caller's workspace; an out-of-workspace or
    // unknown id reads as not-found (no cross-tenant existence oracle).
    let gate = state
        .store
        .get_approval_gate(gate_id)
        .await?
        .filter(|g| auth.bypass || g.workspace_id == auth.workspace_id)
        .ok_or(ApiError::NotFound)?;
    // Integrity of the untrusted `/ui` round-trip: the answer must echo the
    // server-issued token for this gate.
    if !verify_request_state(
        &body.request_state,
        gate.id,
        state.subscribe_resume_secret(),
    ) {
        return Err(ApiError::Forbidden("invalid request_state".into()));
    }
    match state
        .store
        .resolve_approval_gate(gate_id, auth.member_id, target, body.content.as_ref())
        .await?
    {
        Some(resolved) => Ok(Json(resolved)),
        // The CAS found no pending row — already resolved. Silence and
        // double-answers cannot flip an outcome.
        None => Err(ApiError::Conflict("gate is already resolved".into())),
    }
}
