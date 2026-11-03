//! Accepting a workspace invite through the OIDC front door. `start` turns
//! a live invite secret into a pending sign-in bound to the invite's
//! workspace; `complete` runs at the callback, checks the provider's verified
//! email against the invite, and has the store consume the invite and
//! create the human member, identity and session in one commit.

use axum::{http::HeaderMap, response::Response};
use chrono::{Duration, Utc};
use maidan_auth::hash_secret;
use maidan_store::workspace_invites::{INVITE_ALREADY_MEMBER, INVITE_NOT_VALID};
use maidan_types::{
    AcceptWorkspaceInvite, AuditScope, NewAuditEvent, NewOidcPendingAuth, OidcPendingAuth,
    OidcPendingTarget, WorkspaceInviteId,
};
use openidconnect::PkceCodeVerifier;

use super::handlers::{authorize_redirect, finish_with_session, random_token, SESSION_CREATE};
use crate::error::ApiError;
use crate::state::AppState;

/// The audit action of a person joining a workspace by invite.
pub const WORKSPACE_INVITE_ACCEPT: &str = "workspace_invite.accept";

/// What the provider said about the person at the callback.
pub(super) struct Verified {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

fn not_valid() -> ApiError {
    ApiError::Forbidden(INVITE_NOT_VALID.into())
}

/// Begin the provider round trip for an invite. An unknown, used, revoked
/// or expired invite is refused here, in one message, before any redirect.
pub(super) async fn start(
    state: &AppState,
    oidc: &crate::oidc::OidcRuntime,
    secret: &str,
    return_to: Option<String>,
) -> Result<Response, ApiError> {
    let invite = state
        .store
        .find_live_workspace_invite(&hash_secret(secret.trim()), Utc::now())
        .await
        .map_err(|err| match err {
            maidan_store::StoreError::NotFound => not_valid(),
            other => other.into(),
        })?;

    let state_token = random_token();
    let nonce = random_token();
    let pkce_verifier = PkceCodeVerifier::new(random_token());
    let expires_at = Utc::now() + Duration::seconds(oidc.settings.pending_ttl_secs as i64);
    state
        .store
        .insert_oidc_pending(NewOidcPendingAuth {
            state: state_token.clone(),
            target: OidcPendingTarget::Workspace(invite.workspace_id),
            nonce: nonce.clone(),
            pkce_verifier: pkce_verifier.secret().to_string(),
            return_to,
            expires_at,
            invite_entry_id: None,
            workspace_invite_id: Some(invite.id),
        })
        .await?;
    authorize_redirect(oidc, state_token, nonce, &pkce_verifier)
}

/// Finish an invite at the callback. Every check here runs again inside the
/// store's transaction, since the invite can be used, revoked or expire
/// during the round trip; these turn the common refusals away before the
/// domain charge.
pub(super) async fn complete(
    state: &AppState,
    oidc: &crate::oidc::OidcRuntime,
    pending: &OidcPendingAuth,
    invite_id: WorkspaceInviteId,
    who: Verified,
    headers_in: &HeaderMap,
) -> Result<Response, ApiError> {
    let Some(email) = who.email.filter(|_| who.email_verified) else {
        return Err(ApiError::Forbidden(
            "joining by invite needs a verified email".into(),
        ));
    };
    let OidcPendingTarget::Workspace(workspace_id) = pending.target else {
        return Err(not_valid());
    };
    let invite = match state.store.get_workspace_invite(invite_id).await {
        Ok(invite) => invite,
        Err(maidan_store::StoreError::NotFound) => return Err(not_valid()),
        Err(err) => return Err(err.into()),
    };
    if invite.workspace_id != workspace_id
        || invite.accepted_at.is_some()
        || invite.revoked_at.is_some()
        || invite.expires_at <= Utc::now()
    {
        return Err(not_valid());
    }
    if !invite.email.eq_ignore_ascii_case(email.trim()) {
        return Err(ApiError::Forbidden(
            "the invite was issued to a different email".into(),
        ));
    }

    crate::rate_limit::charge_email_domain(
        state,
        crate::rate_limit::DomainCharge::WorkspaceInviteAccept,
        &email,
    )
    .await?;

    let handle = crate::oidc::member::handle_from_claims(&who.subject, Some(&email));
    let issuer = who.issuer.clone();
    let session_expires_at = Utc::now() + Duration::seconds(oidc.settings.session_ttl_secs as i64);
    let accepted = state
        .store
        .accept_workspace_invite(
            AcceptWorkspaceInvite {
                invite_id,
                workspace_id,
                email,
                issuer: who.issuer,
                subject: who.subject,
                handle,
                session_expires_at,
            },
            Box::new(move |done| NewAuditEvent {
                scope: AuditScope::Workspace(done.invite.workspace_id),
                actor_id: Some(done.member.id),
                action: WORKSPACE_INVITE_ACCEPT.into(),
                target_kind: Some("workspace_invite".into()),
                target_id: Some(done.invite.id.0),
                metadata: serde_json::json!({
                    "workspace_id": done.invite.workspace_id.0,
                    "member_id": done.member.id.0,
                    "role": done.invite.role.as_str(),
                    "invited_by": done.invite.created_by.0,
                    "issuer": issuer,
                }),
            }),
            Box::new(move |session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: SESSION_CREATE.into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: serde_json::json!({
                    "workspace_id": session.workspace_id.0,
                    "member": "invite",
                    "expires_at": session.expires_at,
                }),
            }),
        )
        .await
        .map_err(|err| match err {
            maidan_store::StoreError::Conflict(msg) if msg == INVITE_ALREADY_MEMBER => {
                ApiError::Forbidden(msg)
            }
            maidan_store::StoreError::Conflict(_) | maidan_store::StoreError::NotFound => {
                not_valid()
            }
            other => other.into(),
        })?;

    finish_with_session(
        state,
        oidc,
        headers_in,
        &accepted.session,
        pending.return_to.as_deref(),
    )
    .await
}
