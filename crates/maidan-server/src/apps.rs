//! Installed app registry and app-scoped API tokens (Cluster 57.0).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{self, validate_list, validate_subset},
    hash_secret, AuthContext, TokenSecret,
};
use maidan_types::*;

use crate::dto::{
    AppInstallationResponse, AppResponse, InstallApp, MintAppToken, MintAppTokenResponse,
    RegisterApp,
};
use crate::error::{ApiError, ApiJson};
use crate::routes::{cap, ensure_workspace};
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

pub async fn register_app(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<RegisterApp>,
) -> ApiResult<(StatusCode, Json<AppResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, capability::WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;

    let slug = normalize_slug(&body.slug)?;
    let app = state
        .store
        .create_app(NewApp {
            workspace_id,
            slug,
            name: body.name,
            description: body.description,
            created_by: auth.member_id,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(AppResponse::from(app))))
}

pub async fn list_apps(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<AppResponse>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, capability::WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let apps = state.store.list_apps(workspace_id).await?;
    Ok(Json(apps.into_iter().map(AppResponse::from).collect()))
}

pub async fn install_app(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, app_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<InstallApp>,
) -> ApiResult<(StatusCode, Json<AppInstallationResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    let app_id = AppId(app_id);
    cap(&auth, capability::TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;

    let app = state.store.get_app(app_id).await?;
    if app.workspace_id != workspace_id {
        return Err(ApiError::BadRequest("app is not in this workspace".into()));
    }

    let granted = if body.granted_capabilities.is_empty() {
        capability::default_minted()
    } else {
        validate_list(&body.granted_capabilities).map_err(ApiError::BadRequest)?;
        body.granted_capabilities
    };

    let handle = format!("app:{}", app.slug);
    let bot = state
        .store
        .create_member(NewMember {
            workspace_id,
            handle,
            display_name: Some(app.name.clone()),
            kind: MemberKind::Agent,
        })
        .await?;

    let installation = state
        .store
        .create_app_installation(NewAppInstallation {
            app_id: app.id,
            workspace_id,
            bot_member_id: bot.id,
            granted_capabilities: granted,
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(AppInstallationResponse::from(installation)),
    ))
}

pub async fn list_app_installations(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<AppInstallationResponse>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, capability::WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let rows = state.store.list_app_installations(workspace_id).await?;
    Ok(Json(
        rows.into_iter()
            .map(AppInstallationResponse::from)
            .collect(),
    ))
}

pub async fn revoke_app_installation(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, installation_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<AppInstallationResponse>> {
    let workspace_id = WorkspaceId(workspace_id);
    let installation_id = AppInstallationId(installation_id);
    cap(&auth, capability::TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;

    let row = state.store.get_app_installation(installation_id).await?;
    if row.workspace_id != workspace_id {
        return Err(ApiError::NotFound);
    }
    let revoked = state.store.revoke_app_installation(installation_id).await?;
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "app_installation.revoke".into(),
            target_kind: Some("app_installation".into()),
            target_id: Some(installation_id.0),
            metadata: serde_json::json!({ "workspace_id": workspace_id.0 }),
        },
    )
    .await;
    Ok(Json(AppInstallationResponse::from(revoked)))
}

pub async fn mint_app_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, installation_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<MintAppToken>,
) -> ApiResult<(StatusCode, Json<MintAppTokenResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    let installation_id = AppInstallationId(installation_id);
    cap(&auth, capability::TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;

    let installation = state.store.get_app_installation(installation_id).await?;
    if installation.workspace_id != workspace_id {
        return Err(ApiError::NotFound);
    }
    if installation.revoked_at.is_some() {
        return Err(ApiError::BadRequest("app installation is revoked".into()));
    }

    let capabilities = if body.capabilities.is_empty() {
        installation.granted_capabilities.clone()
    } else {
        validate_subset(&installation.granted_capabilities, &body.capabilities)
            .map_err(ApiError::BadRequest)?;
        body.capabilities
    };
    crate::quota::validate_token_quotas(&body.quotas, &capabilities)?;

    let secret = TokenSecret::generate();
    let record = state
        .store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id: installation.bot_member_id,
            app_installation_id: Some(installation_id),
            token_hash: hash_secret(secret.as_str()),
            label: body.label,
            capabilities: capabilities.clone(),
            expires_at: body.expires_at,
        })
        .await?;

    if !body.quotas.is_empty() {
        state
            .store
            .replace_token_quotas(record.id, &body.quotas)
            .await?;
    }
    let quotas = state.store.list_token_quotas(record.id).await?;

    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "app_token.mint".into(),
            target_kind: Some("api_token".into()),
            target_id: Some(record.id.0),
            metadata: serde_json::json!({
                "workspace_id": record.workspace_id.0,
                "app_installation_id": installation_id.0,
                "bot_member_id": installation.bot_member_id.0,
                "capabilities": capabilities.clone(),
                "expires_at": record.expires_at,
            }),
        },
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(MintAppTokenResponse {
            id: record.id,
            secret: secret.as_str().to_string(),
            workspace_id: record.workspace_id,
            app_installation_id: installation_id,
            bot_member_id: installation.bot_member_id,
            capabilities,
            expires_at: record.expires_at,
            quotas,
        }),
    ))
}

fn normalize_slug(slug: &str) -> Result<String, ApiError> {
    let s = slug.trim().to_ascii_lowercase();
    if s.is_empty() || s.len() > 64 {
        return Err(ApiError::BadRequest("slug must be 1–64 characters".into()));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::BadRequest(
            "slug may only contain letters, digits, hyphen, underscore".into(),
        ));
    }
    Ok(s)
}
