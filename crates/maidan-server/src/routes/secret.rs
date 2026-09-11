//! Named-secret management (Cluster 371, Wave 2 #19, G19/T3): create/rotate,
//! list (metadata only), **resolve** (the value — "Pi fetches at exec"), delete.
//! The value is AEAD-encrypted at rest with the Cluster-189 keyring, which the
//! route layer holds; it appears only in a create request and a resolve response,
//! never in the event log. `secret:admin` writes; `secret:read` reads/resolves.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{SECRET_ADMIN, SECRET_READ},
    decrypt_peer_secret_rotating, encrypt_peer_secret, AuthContext,
};
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

/// The at-rest encryption key, or a clear error when the deployment hasn't
/// configured one — secrets can't be stored/read without it.
fn require_key(state: &AppState) -> ApiResult<&[u8; 32]> {
    state.federation.encryption_key.as_deref().ok_or_else(|| {
        ApiError::Internal(
            "secret storage requires an encryption key (set FEDERATION_ENCRYPTION_KEY)".into(),
        )
    })
}

fn validate_name(name: &str) -> ApiResult<()> {
    if is_valid_secret_name(name) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "secret name must be non-empty and use only [A-Za-z0-9_.-]".into(),
        ))
    }
}

pub async fn create_secret(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateSecret>,
) -> ApiResult<(StatusCode, Json<Secret>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SECRET_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    validate_name(&body.name)?;
    if body.value.is_empty() {
        return Err(ApiError::BadRequest(
            "secret value must not be empty".into(),
        ));
    }
    let key = require_key(&state)?;
    let value_ciphertext =
        encrypt_peer_secret(&body.value, key).map_err(|e| ApiError::Internal(e.to_string()))?;
    let secret = state
        .store
        .create_secret(NewSecret {
            workspace_id,
            name: body.name,
            value_ciphertext,
            created_by: auth.member_id,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(secret)))
}

pub async fn list_secrets(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Secret>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SECRET_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_secrets(workspace_id).await?))
}

pub async fn resolve_secret(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, name)): Path<(uuid::Uuid, String)>,
) -> ApiResult<Json<SecretValue>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SECRET_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let ciphertext = state
        .store
        .get_secret_ciphertext(workspace_id, &name)
        .await?
        .ok_or(ApiError::NotFound)?;
    let key = require_key(&state)?;
    let value = decrypt_peer_secret_rotating(&ciphertext, key)
        .map_err(|e| ApiError::Internal(format!("secret decrypt failed: {e}")))?;
    Ok(Json(SecretValue { name, value }))
}

pub async fn delete_secret(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, name)): Path<(uuid::Uuid, String)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SECRET_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    if state.store.delete_secret(workspace_id, &name).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
