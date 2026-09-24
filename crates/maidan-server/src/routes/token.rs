//! API-token handlers: mint, list, revoke, and holder-side attenuation.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use maidan_auth::{
    capability::{self, TOKEN_ADMIN, WORKSPACE_READ},
    hash_secret, AuthContext, TokenSecret,
};
use maidan_store::Store;
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

fn holder_grant(auth: &AuthContext) -> Vec<String> {
    if auth.bypass {
        capability::all()
    } else {
        auth.capabilities().to_vec()
    }
}

async fn parent_expires_at(
    store: &dyn Store,
    auth: &AuthContext,
) -> ApiResult<Option<DateTime<Utc>>> {
    match auth.token_id {
        Some(id) => Ok(store.get_api_token(id).await?.expires_at),
        None => Ok(None),
    }
}

pub async fn mint_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, member_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<MintApiToken>,
) -> ApiResult<(StatusCode, Json<MintApiTokenResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    let member_id = MemberId(member_id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;

    let member = state.store.get_member(member_id).await?;
    if member.workspace_id != workspace_id {
        return Err(ApiError::BadRequest(
            "member does not belong to workspace".into(),
        ));
    }

    // token:admin may grant any known set or list (held = vocabulary).
    // Holder-side attenuation is POST /tokens/attenuate — no token:admin.
    let capabilities = if body.capability_set.is_none() && body.capabilities.is_empty() {
        capability::default_minted()
    } else {
        maidan_auth::progressive_grant(
            &capability::all(),
            body.capability_set.as_deref(),
            &body.capabilities,
        )
        .map_err(ApiError::BadRequest)?
    };
    crate::quota::validate_token_quotas(&body.quotas, &capabilities)?;

    let secret = TokenSecret::generate();
    let actor = auth.actor_id;
    let capability_set = body.capability_set.clone();
    let record = state
        .store
        .create_api_token_audited(
            NewApiToken {
                workspace_id,
                member_id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: body.label,
                capabilities: capabilities.clone(),
                expires_at: body.expires_at,
            },
            Box::new(move |record| NewAuditEvent {
                actor_id: Some(actor),
                action: "token.mint".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(record.id.0),
                metadata: serde_json::json!({
                    "workspace_id": record.workspace_id.0,
                    "subject_member_id": record.member_id.0,
                    "capabilities": record.capabilities.clone(),
                    "capability_set": capability_set,
                    "expires_at": record.expires_at,
                }),
            }),
        )
        .await?;

    if !body.quotas.is_empty() {
        state
            .store
            .replace_token_quotas(record.id, &body.quotas)
            .await?;
    }
    let quotas = state.store.list_token_quotas(record.id).await?;

    Ok((
        StatusCode::CREATED,
        Json(MintApiTokenResponse {
            id: record.id,
            secret: secret.as_str().to_string(),
            workspace_id: record.workspace_id,
            member_id: record.member_id,
            capabilities: record.capabilities,
            expires_at: record.expires_at,
            quotas,
        }),
    ))
}

pub async fn list_api_tokens(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, member_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<Vec<crate::dto::ApiTokenSummary>>> {
    let workspace_id = WorkspaceId(workspace_id);
    let member_id = MemberId(member_id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let member = state.store.get_member(member_id).await?;
    if member.workspace_id != workspace_id {
        return Err(ApiError::BadRequest(
            "member does not belong to workspace".into(),
        ));
    }
    let tokens = state
        .store
        .list_api_tokens_for_member(workspace_id, member_id)
        .await?;
    Ok(Json(
        tokens
            .into_iter()
            .map(|t| crate::dto::ApiTokenSummary {
                id: t.id,
                workspace_id: t.workspace_id,
                member_id: t.member_id,
                label: t.label,
                capabilities: t.capabilities,
                created_at: t.created_at,
                expires_at: t.expires_at,
                revoked_at: t.revoked_at,
            })
            .collect(),
    ))
}

pub async fn revoke_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<ApiToken>> {
    cap(&auth, TOKEN_ADMIN)?;
    let token_id = ApiTokenId(id);
    let existing = state.store.get_api_token(token_id).await?;
    ensure_workspace(&auth, existing.workspace_id)?;
    let actor = auth.actor_id;
    let revoked = state
        .store
        .revoke_api_token_audited(
            token_id,
            Box::new(move |revoked| NewAuditEvent {
                actor_id: Some(actor),
                action: "token.revoke".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(revoked.id.0),
                metadata: serde_json::json!({
                    "workspace_id": revoked.workspace_id.0,
                    "subject_member_id": revoked.member_id.0,
                }),
            }),
        )
        .await?;
    Ok(Json(revoked))
}

/// Named capability-set catalog (`maidan.agent.worker`, `maidan.human.admin`).
pub async fn list_capability_sets(
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<Vec<CapabilitySetView>>> {
    cap(&auth, WORKSPACE_READ)?;
    Ok(Json(
        maidan_auth::named_sets()
            .into_iter()
            .map(|s| CapabilitySetView {
                name: s.name,
                capabilities: s.capabilities,
            })
            .collect(),
    ))
}

/// Levy/Madden holder-side attenuation: derive a weaker token for the caller.
/// No `token:admin`. Amplification is rejected; a derived expiry cannot outlive
/// the parent bearer.
///
/// **A derived token inherits every limit the parent carried**.
/// Attenuation is allowed to be a no-op re-issue — `attenuate` permits an equal
/// capability list — so anything the parent was bound by and the child was not
/// became a way to shed that bound by re-issuing:
///
/// - `app_installation_id` was dropped, so an installed third-party app could
///   derive a standalone member token and keep it after the workspace revoked
///   the installation. `get_active_by_hash` refuses a token whose installation is
///   revoked, and the derived one no longer named an installation to check.
/// - Per-token quotas were dropped, so a token throttled to N calls a minute
///   could mint a functionally identical one with no quota at all.
///
/// Neither bound is something the holder should be able to shed by asking.
pub async fn attenuate_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<AttenuateToken>,
) -> ApiResult<(StatusCode, Json<MintApiTokenResponse>)> {
    cap(&auth, WORKSPACE_READ)?;
    let capabilities = maidan_auth::attenuate(&holder_grant(&auth), &body.capabilities)
        .map_err(ApiError::BadRequest)?;
    let parent = parent_expires_at(state.store.as_ref(), &auth).await?;
    let expires_at = maidan_auth::attenuate_expiry(parent, body.expires_at, Utc::now())
        .map_err(ApiError::BadRequest)?;

    // Quotas are keyed on the token id, so the child needs its own copies of the
    // parent's. Read them before minting: a child that outlives this call with no
    // quota row is the amplification the parent's quota existed to stop.
    let inherited_quotas = match auth.token_id {
        Some(parent_id) => state.store.list_token_quotas(parent_id).await?,
        None => Vec::new(),
    };

    let secret = TokenSecret::generate();
    let derived = NewApiToken {
        workspace_id: auth.workspace_id,
        member_id: auth.member_id,
        // Inherited, not dropped: an app's grant dies with its installation,
        // and a derived token is still that app acting.
        app_installation_id: auth.app_installation_id,
        token_hash: hash_secret(secret.as_str()),
        label: body.label,
        capabilities: capabilities.clone(),
        expires_at,
    };
    // Record the parent so revoking it reaches this token. A holder without a
    // token id is a session, which has nothing to derive from — that case
    // cannot reach here, but it mints unlinked rather than guessing a parent.
    let actor = auth.actor_id;
    let parent_token_id = auth.token_id;
    let quota_count = inherited_quotas.len();
    let audit: maidan_store::AuditFor<ApiToken> = Box::new(move |record| NewAuditEvent {
        actor_id: Some(actor),
        action: "token.mint".into(),
        target_kind: Some("api_token".into()),
        target_id: Some(record.id.0),
        metadata: serde_json::json!({
            "workspace_id": record.workspace_id.0,
            "subject_member_id": record.member_id.0,
            "capabilities": record.capabilities.clone(),
            "expires_at": record.expires_at,
            "attenuated": true,
            "parent_token_id": parent_token_id.map(|t| t.0),
            "app_installation_id": record.app_installation_id.map(|a| a.0),
            "inherited_quotas": quota_count,
        }),
    });
    let record = match auth.token_id {
        Some(parent) => {
            state
                .store
                .create_attenuated_api_token_audited(derived, parent, audit)
                .await?
        }
        None => state.store.create_api_token_audited(derived, audit).await?,
    };
    if !inherited_quotas.is_empty() {
        state
            .store
            .replace_token_quotas(record.id, &inherited_quotas)
            .await?;
    }

    Ok((
        StatusCode::CREATED,
        Json(MintApiTokenResponse {
            id: record.id,
            secret: secret.as_str().to_string(),
            workspace_id: record.workspace_id,
            member_id: record.member_id,
            capabilities: record.capabilities,
            expires_at: record.expires_at,
            quotas: inherited_quotas,
        }),
    ))
}

/// Exchange a durable grant for a short-lived bearer that acts as its subject.
/// The result is the intersection of grant scope, the delegate's current
/// authority, and any explicit further attenuation in the request.
pub async fn delegate_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<DelegateToken>,
) -> ApiResult<(StatusCode, Json<DelegateTokenResponse>)> {
    cap(&auth, WORKSPACE_READ)?;
    // One hop. A borrowed token acts *as* its subject, so letting it exchange
    // would let it use the subject's own grants: A acting as B could become C,
    // and every record of the new token would name B as the delegate — erasing
    // A from the chain. Delegation is exchanged by the real delegate, directly.
    if auth.delegation_grant_id.is_some() {
        return Err(ApiError::Forbidden(
            "a delegated token cannot exchange a grant; delegation is one hop".into(),
        ));
    }
    let grant_id = DelegationGrantId(body.grant_id);
    let grant = state.store.get_delegation_grant(grant_id).await?;
    // Another workspace's grant reads as absent, as on revoke: a 403 here would
    // confirm that the id exists somewhere.
    if !auth.bypass && grant.workspace_id != auth.workspace_id {
        return Err(ApiError::NotFound);
    }
    if grant.delegate_id != auth.actor_id {
        return Err(ApiError::Forbidden(
            "delegation grant belongs to a different delegate".into(),
        ));
    }
    let now = Utc::now();
    if grant.revoked_at.is_some() || grant.expires_at <= now {
        return Err(ApiError::Unauthorized);
    }
    let held = holder_grant(&auth);
    let requested = if body.capabilities.is_empty() {
        grant
            .capabilities
            .iter()
            .filter(|capability| held.contains(capability))
            .cloned()
            .collect()
    } else {
        body.capabilities
    };
    let capabilities = maidan_auth::attenuate(&grant.capabilities, &requested)
        .and_then(|caps| maidan_auth::attenuate(&held, &caps))
        .map_err(ApiError::BadRequest)?;
    let parent_expiry = parent_expires_at(state.store.as_ref(), &auth).await?;
    let expires_at =
        maidan_auth::delegated_expiry(grant.expires_at, parent_expiry, body.expires_at, now)
            .map_err(ApiError::BadRequest)?;

    let secret = TokenSecret::generate();
    let (actor, delegate, parent_token_id, grant_id) =
        (auth.actor_id, auth.member_id, auth.token_id, grant.id);
    let record = state
        .store
        .create_delegated_api_token_audited(
            NewApiToken {
                workspace_id: grant.workspace_id,
                member_id: grant.subject_id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: body.label,
                capabilities: capabilities.clone(),
                expires_at: Some(expires_at),
            },
            grant.id,
            auth.member_id,
            auth.token_id,
            Box::new(move |record| NewAuditEvent {
                actor_id: Some(actor),
                action: "token.delegate".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(record.id.0),
                metadata: serde_json::json!({
                    "workspace_id": record.workspace_id.0,
                    "delegate_id": delegate.0,
                    "subject_member_id": record.member_id.0,
                    "grant_id": grant_id.0,
                    "capabilities": record.capabilities.clone(),
                    "expires_at": record.expires_at,
                    "parent_token_id": parent_token_id.map(|id| id.0),
                }),
            }),
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(DelegateTokenResponse {
            grant_id: grant.id,
            delegate_id: auth.member_id,
            token: MintApiTokenResponse {
                id: record.id,
                secret: secret.as_str().to_owned(),
                workspace_id: record.workspace_id,
                member_id: record.member_id,
                capabilities: record.capabilities,
                expires_at: record.expires_at,
                quotas: Vec::new(),
            },
        }),
    ))
}

pub async fn create_delegation_grant(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateDelegationGrant>,
) -> ApiResult<(StatusCode, Json<DelegationGrant>)> {
    cap(&auth, TOKEN_ADMIN)?;
    let workspace_id = WorkspaceId(workspace_id);
    ensure_workspace(&auth, workspace_id)?;
    if let Some(unknown) = body
        .capabilities
        .iter()
        .find(|capability| !capability::is_known(capability))
    {
        return Err(ApiError::BadRequest(format!(
            "unknown delegated capability: {unknown}"
        )));
    }
    if let Some(authority) = body
        .capabilities
        .iter()
        .find(|capability| !capability::is_delegatable(capability))
    {
        return Err(ApiError::BadRequest(format!(
            "{authority} cannot be delegated: a grant lends the ability to do work, \
             never the means to hand out more authority"
        )));
    }
    let subject_id = MemberId(body.subject_id);
    let delegate_id = MemberId(body.delegate_id);
    for member_id in [subject_id, delegate_id] {
        let member = state.store.get_member(member_id).await?;
        if member.workspace_id != workspace_id {
            return Err(ApiError::BadRequest(
                "subject and delegate must belong to the workspace".into(),
            ));
        }
    }
    let actor = auth.actor_id;
    let grant = state
        .store
        .create_delegation_grant_audited(
            NewDelegationGrant {
                workspace_id,
                subject_id,
                delegate_id,
                capabilities: body.capabilities,
                purpose: body.purpose,
                authorized_by: auth.actor_id,
                expires_at: body.expires_at,
            },
            Box::new(move |grant| NewAuditEvent {
                actor_id: Some(actor),
                action: "delegation_grant.create".into(),
                target_kind: Some("delegation_grant".into()),
                target_id: Some(grant.id.0),
                metadata: serde_json::json!({
                    "workspace_id": workspace_id.0,
                    "subject_id": grant.subject_id.0,
                    "delegate_id": grant.delegate_id.0,
                    "capabilities": grant.capabilities.clone(),
                    "expires_at": grant.expires_at,
                    "purpose": grant.purpose.clone(),
                }),
            }),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(grant)))
}

pub async fn list_delegation_grants(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<DelegationGrant>>> {
    cap(&auth, TOKEN_ADMIN)?;
    let workspace_id = WorkspaceId(workspace_id);
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        state.store.list_delegation_grants(workspace_id).await?,
    ))
}

pub async fn revoke_delegation_grant(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, grant_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<DelegationGrant>> {
    cap(&auth, TOKEN_ADMIN)?;
    let workspace_id = WorkspaceId(workspace_id);
    let grant_id = DelegationGrantId(grant_id);
    ensure_workspace(&auth, workspace_id)?;
    let existing = state.store.get_delegation_grant(grant_id).await?;
    if existing.workspace_id != workspace_id {
        return Err(ApiError::NotFound);
    }
    state
        .store
        .revoke_delegation_grant_audited(
            workspace_id,
            grant_id,
            NewAuditEvent {
                actor_id: Some(auth.actor_id),
                action: "delegation_grant.revoke".into(),
                target_kind: Some("delegation_grant".into()),
                target_id: Some(grant_id.0),
                metadata: serde_json::json!({
                    "workspace_id": workspace_id.0,
                    "subject_id": existing.subject_id.0,
                    "delegate_id": existing.delegate_id.0,
                }),
            },
        )
        .await?;
    let grant = state.store.get_delegation_grant(grant_id).await?;
    Ok(Json(grant))
}
