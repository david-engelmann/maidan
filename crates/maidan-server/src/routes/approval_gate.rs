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
    capability::{APPROVAL_GRANT, TOKEN_ADMIN, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_store::approval_policy::CONFIRM_LINK_TTL_SECONDS_RANGE;
use maidan_types::{
    ApprovalGate, ApprovalGateId, ApprovalGateState, ApprovalPolicy, AuditScope, ConfirmOutcome,
    MemberKind, NewAuditEvent, WorkspaceId,
};
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
    let mut requests = state
        .store
        .list_live_approval_confirmations(workspace_id, chrono::Utc::now())
        .await?;
    let views = gates
        .into_iter()
        .map(|gate| {
            let request_state = sign_request_state(gate.id, secret);
            let model_request = requests
                .iter()
                .position(|c| c.gate_id == gate.id)
                .map(|at| requests.swap_remove(at))
                .map(|c| ModelRequestView {
                    member_id: c.member_id,
                    client_name: c.client_name,
                    client_version: c.client_version,
                    client_id: c.client_id,
                    client_source: c.client_source,
                    expires_at: c.expires_at,
                });
            ApprovalGateView {
                gate,
                request_state,
                model_request,
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

/// `PUT /workspaces/:id/approval-policy`: the lowest gate risk at which a
/// model's accept through `approval_decide` needs a person to confirm it, or
/// `null` for the default, `low` (every accept through the tool), and how
/// long the confirmation link lives, 60 to 3600 seconds, or `null` for 600.
/// It decides when a token may accept without a person, so it is
/// `token:admin`, like the delegation policy. Audited in its own transaction.
pub async fn set_approval_policy(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<SetApprovalPolicy>,
) -> ApiResult<Json<ApprovalPolicy>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let ttl = body
        .confirm_link_ttl_seconds
        .map(|ttl| {
            u32::try_from(ttl)
                .ok()
                .filter(|t| CONFIRM_LINK_TTL_SECONDS_RANGE.contains(t))
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "confirm_link_ttl_seconds must be from {} to {}",
                        CONFIRM_LINK_TTL_SECONDS_RANGE.start(),
                        CONFIRM_LINK_TTL_SECONDS_RANGE.end()
                    ))
                })
        })
        .transpose()?;
    let actor = auth.actor_id;
    let policy = state
        .store
        .set_approval_policy_audited(
            workspace_id,
            body.confirm_at,
            ttl,
            Box::new(move |policy| NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(actor),
                action: "approval_policy.set".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(workspace_id.0),
                metadata: serde_json::json!({
                    "confirm_at": policy.confirm_at,
                    "confirm_link_ttl_seconds": policy.confirm_link_ttl_seconds,
                    "is_default": policy.is_default,
                }),
            }),
        )
        .await?;
    Ok(Json(policy))
}

/// `GET /workspaces/:id/approval-policy`: the threshold and link lifetime in
/// force.
/// `workspace:read`: a member may know when a model's accept needs them.
pub async fn get_approval_policy(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<ApprovalPolicy>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.get_approval_policy(workspace_id).await?))
}

/// Why a confirmation was refused before its link was looked at.
const CONFIRM_NEEDS: &str = "confirming a model's approval request needs the signed-in console \
     session of the person it was sent to, not a bearer token";

/// `POST /auth/approval-confirmations/confirm`: a person confirms a model's
/// request to accept a gate, from the link `approval_decide` gave them.
///
/// It is on the session-only tree: the request has to carry the session
/// cookie, come from the console page (the strict origin check), and
/// pass [`acceptance_credential`] as a human member. It must be a session the
/// person signed in to: one made from a token is refused, since the model
/// holding that token could otherwise finish the confirmation itself. The link is
/// bound to the member whose credential the model used, so only that person's
/// session can spend it, and nobody confirms acceptance of their own request.
/// The gate is recorded as decided via the client the model asked through.
///
/// A link that is unknown, someone else's, for another workspace, used or
/// expired is one 404, so the route says nothing about any link but a live
/// one of the caller's own.
pub async fn confirm_approval_gate(
    State(state): State<AppState>,
    Extension(session): Extension<SessionContext>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ConfirmApprovalGate>,
) -> ApiResult<Json<ApprovalGate>> {
    // Only a person's signed-in console session confirms. A session minted
    // from a token carries that token's authority, so a model holding the
    // token could otherwise exchange it and finish the confirmation itself,
    // approval:grant or not.
    if session.token.is_some() {
        return Err(ApiError::Forbidden(CONFIRM_NEEDS.into()));
    }
    // The person's authority on this route: what an OIDC session writes with.
    let auth = AuthContext::from_session(
        session.member_id,
        session.workspace_id,
        vec![WORKSPACE_READ.into(), WORKSPACE_WRITE.into()],
    );
    require_same_origin(&headers)?;
    let token_hash = maidan_auth::approval_confirmation::token_hash(body.token.trim());
    let confirmation = state
        .store
        .get_approval_confirmation_by_token(&token_hash)
        .await?
        .filter(|c| {
            c.gate_id == body.gate_id
                && c.workspace_id == auth.workspace_id
                && c.member_id == auth.member_id
        })
        .ok_or(ApiError::NotFound)?;
    let gate = state
        .store
        .get_approval_gate(confirmation.gate_id)
        .await?
        .filter(|g| g.workspace_id == auth.workspace_id)
        .ok_or(ApiError::NotFound)?;
    let asked = [Some(gate.requested_by), gate.requested_actor_id];
    let answering = [auth.member_id, auth.actor_id];
    if answering.iter().any(|m| asked.contains(&Some(*m))) {
        return Err(ApiError::Forbidden(
            "an approval cannot be granted by whoever requested it".into(),
        ));
    }
    acceptance_credential(&state, &auth, Some(&session), &headers).await?;
    let actor = auth.actor_id;
    let outcome = state
        .store
        .confirm_approval_gate(
            &token_hash,
            auth.workspace_id,
            auth.member_id,
            chrono::Utc::now(),
            Box::new(move |g: &ApprovalGate| NewAuditEvent {
                scope: AuditScope::Workspace(g.workspace_id),
                actor_id: Some(actor),
                action: "approval_gate.decided".into(),
                target_kind: Some("approval_gate".into()),
                target_id: Some(g.id.0),
                metadata: serde_json::json!({
                    "surface": "console",
                    "tool": "approval_decide",
                    "decision": g.state,
                    "path": "confirmation",
                    "risk": g.risk,
                    "client_name": g.decided_via.as_ref().and_then(|v| v.client_name.clone()),
                    "client_version": g.decided_via.as_ref().and_then(|v| v.client_version.clone()),
                    "client_id": g.decided_via.as_ref().and_then(|v| v.client_id.clone()),
                    "client_source": g.decided_via.as_ref().map(|v| v.client_source),
                    "model_asked": true,
                }),
            }),
        )
        .await?;
    match outcome {
        ConfirmOutcome::Accepted(gate) => Ok(Json(*gate)),
        ConfirmOutcome::NotFound => Err(ApiError::NotFound),
        ConfirmOutcome::GateResolved => Err(ApiError::Conflict("gate is already resolved".into())),
    }
}
