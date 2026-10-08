//! Approval-gate REST surface: the human side of the held gate.
//!
//! A workspace member lists the pending gates (each carrying a server-issued
//! `request_state`) and answers one accept/decline/cancel. The `request_state`
//! is an HMAC over the gate id — the `/ui` is an untrusted client, so the
//! answer must echo a token the server actually issued, verified before the
//! resolve. The resolve is a compare-and-set on `pending`, so a second answer —
//! or a late answer after cancel — is a no-op (silence is not consent).
//!
//! Accepting is a property of the credential, not of the member: a bearer
//! token whose member is a human is exactly the token a person hands an agent,
//! so it proves nothing about who decided. See [`acceptance_credential`].

use axum::{extract::State, http::HeaderMap, Extension, Json};
use hmac::{Hmac, Mac};
use maidan_auth::{
    capability::{APPROVAL_GRANT, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::{ApprovalGate, ApprovalGateId, ApprovalGateState, MemberKind, WorkspaceId};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::session::{require_same_origin, SessionContext};
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

/// What a refused accept says: the capability it lacks, in the words every
/// capability refusal uses, then the other way in.
const ACCEPT_NEEDS: &str = "missing capability: approval:grant. Accepting an approval gate \
     needs a browser session a person signed in to, or a token holding approval:grant. A \
     bearer token, or a session made from one, can decline or cancel a gate but not accept it";

/// Whether this request's credential may accept a gate: a token (or a session
/// made from one) holding `approval:grant`, or a browser session a person
/// signed in to through the identity provider, sent from the console page.
///
/// A session made from a token does not count on its own. Anyone holding the
/// token can make one with `POST /auth/session/from-token`, so it proves no
/// more than the token; it carries that token's authority, `approval:grant`
/// included or not. A signed-in session's cookie is `HttpOnly` and comes from
/// the identity provider's redirect, which a model holding a person's token
/// cannot complete. The origin check is the strict one: a request naming no
/// origin is the non-browser client the cookie fallback exists for, and is
/// refused here.
async fn acceptance_credential(
    state: &AppState,
    auth: &AuthContext,
    session: Option<&SessionContext>,
    headers: &HeaderMap,
) -> ApiResult<()> {
    if auth.has_capability(APPROVAL_GRANT) {
        return Ok(());
    }
    let signed_in = session.is_some_and(|s| s.token.is_none()) && auth.token_id.is_none();
    if !signed_in {
        return Err(ApiError::Forbidden(ACCEPT_NEEDS.into()));
    }
    require_same_origin(headers)?;
    // A signed-in session is a person's, but the member it names is checked
    // too: human-control state is answered by a human.
    let member = state.store.get_member(auth.member_id).await?;
    if member.kind != MemberKind::Human {
        return Err(ApiError::Forbidden(
            "an approval gate is accepted by a human member, or by a token granted \
             approval:grant"
                .into(),
        ));
    }
    Ok(())
}

/// List a workspace's pending approval gates, each with its `request_state`.
/// `workspace:read`.
pub async fn list_approval_gates(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<ApprovalGateView>>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(wid);
    ensure_workspace(&auth, workspace_id)?;
    // A gate list without a signing key cannot carry the `requestState` the
    // answer route requires, so an unsigned list would be a dead end. 500 is
    // the honest answer: the deployment is misconfigured.
    let secret = state
        .subscribe_resume_secret()
        .ok_or_else(|| ApiError::Internal("approval gate signing key not configured".into()))?;
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

/// Answer a pending approval gate — accept / decline / cancel.
/// `workspace:write`. The `request_state` is integrity-verified and the gate
/// must be in the caller's workspace. Accepting also needs the credential
/// [`acceptance_credential`] names. Resolve is a CAS on `pending`, so a second
/// answer is a no-op → `409`.
pub async fn answer_approval_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    headers: HeaderMap,
    ApiPath(id): ApiPath<uuid::Uuid>,
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
    // No signing key means no `requestState` could have been issued, so nothing
    // can legitimately verify — fail closed rather than waving the answer through.
    let secret = state
        .subscribe_resume_secret()
        .ok_or_else(|| ApiError::Internal("approval gate signing key not configured".into()))?;
    if !verify_request_state(&body.request_state, gate.id, secret) {
        return Err(ApiError::Forbidden("invalid request_state".into()));
    }
    // Nobody accepts a gate they asked for, whichever identity they asked or
    // answer under, and whatever credential they hold. Declining or
    // cancelling your own request is not approving it, and stays open.
    if target == ApprovalGateState::Accepted && !auth.bypass {
        let asked = [Some(gate.requested_by), gate.requested_actor_id];
        let answering = [auth.member_id, auth.actor_id];
        if answering.iter().any(|m| asked.contains(&Some(*m))) {
            return Err(ApiError::Forbidden(
                "an approval cannot be granted by whoever requested it".into(),
            ));
        }
        // An approval gate is human-control state, and a worker holds the
        // `workspace:write` this route asks for: without this, agent B could
        // accept agent A's gate, and an agent holding a person's token could
        // accept any gate with curl.
        acceptance_credential(&state, &auth, session.as_deref(), &headers).await?;
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
