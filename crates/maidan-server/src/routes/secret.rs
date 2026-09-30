//! Named-secret management: create/rotate, list (metadata only), **resolve**
//! (the value — "a consumer fetches at exec"), delete. The value is
//! AEAD-encrypted at rest with the keyring, which the route layer holds; it
//! appears only in a create request and a resolve response, never in the event
//! log. `secret:admin` writes; `secret:read` reads/resolves.
//!
//! The secret-egress allowlist is here too: the hosts the egress broker may
//! substitute this workspace's secret values for. Adding a host needs
//! `secret:read` as well as `secret:admin`, because a listed host receives the
//! value of any secret a payload bound for it names; without that, a token
//! that may rotate secrets but not read them could read them anyway by
//! listing a host it controls.

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::{
    capability::{SECRET_ADMIN, SECRET_READ},
    decrypt_peer_secret_rotating, encrypt_peer_secret, AuthContext,
};
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
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
    ApiPath(workspace_id): ApiPath<uuid::Uuid>,
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
    let actor = auth.actor_id;
    let secret = state
        .store
        .create_secret_audited(
            NewSecret {
                workspace_id,
                name: body.name,
                value_ciphertext,
                created_by: auth.member_id,
            },
            Box::new(move |secret| NewAuditEvent {
                scope: AuditScope::Workspace(secret.workspace_id),
                actor_id: Some(actor),
                action: "secret.create".into(),
                target_kind: Some("secret".into()),
                target_id: Some(secret.id.0),
                metadata: serde_json::json!({
                    "workspace_id": secret.workspace_id.0,
                    "name": secret.name,
                }),
            }),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(secret)))
}

pub async fn list_secrets(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(workspace_id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<Secret>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SECRET_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_secrets(workspace_id).await?))
}

pub async fn resolve_secret(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((workspace_id, name)): ApiPath<(uuid::Uuid, String)>,
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
    // Recorded before the value is released, and withheld if it cannot be.
    state
        .store
        .append_audit(NewAuditEvent {
            scope: AuditScope::Workspace(workspace_id),
            actor_id: Some(auth.actor_id),
            action: "secret.resolve".into(),
            target_kind: Some("secret".into()),
            target_id: None,
            metadata: serde_json::json!({ "workspace_id": workspace_id.0, "name": name }),
        })
        .await?;
    Ok(Json(SecretValue { name, value }))
}

pub async fn delete_secret(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((workspace_id, name)): ApiPath<(uuid::Uuid, String)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SECRET_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let deleted = state
        .store
        .delete_secret_audited(
            workspace_id,
            &name,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id),
                action: "secret.delete".into(),
                target_kind: Some("secret".into()),
                target_id: None,
                metadata: serde_json::json!({ "workspace_id": workspace_id.0, "name": name }),
            },
        )
        .await?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// `POST /workspaces/:wid/secret-egress-hosts` — trust a host with the
/// workspace's secret values. Idempotent. `400` for a host that is not a bare
/// hostname, or one outside the instance ceiling (`MAIDAN_SECRET_EGRESS_ALLOWLIST`).
pub async fn allow_secret_egress_host(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(workspace_id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<AllowSecretEgressHost>,
) -> ApiResult<(StatusCode, Json<SecretEgressHost>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SECRET_ADMIN)?;
    cap(&auth, SECRET_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let host = normalize_secret_egress_host(&body.host)
        .map_err(|why| ApiError::BadRequest(why.to_string()))?;
    if !within_secret_egress_ceiling(&host, state.mcp.secret_egress_ceiling()) {
        return Err(ApiError::BadRequest(
            "host is outside this instance's secret-egress ceiling (MAIDAN_SECRET_EGRESS_ALLOWLIST)"
                .into(),
        ));
    }
    let actor = auth.actor_id;
    let entry = state
        .store
        .allow_secret_egress_host_audited(
            NewSecretEgressHost { workspace_id, host },
            Box::new(move |entry| NewAuditEvent {
                scope: AuditScope::Workspace(entry.workspace_id),
                actor_id: Some(actor),
                action: "secret_egress_host.allow".into(),
                target_kind: Some("secret_egress_host".into()),
                target_id: None,
                metadata: serde_json::json!({
                    "workspace_id": entry.workspace_id.0,
                    "host": entry.host,
                }),
            }),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(entry)))
}

/// `GET /workspaces/:wid/secret-egress-hosts` — the hosts trusted with the
/// workspace's secret values. Empty is the default and means substitute
/// nowhere. `secret:admin`: the list is policy, and names the hosts worth
/// aiming a payload at.
pub async fn list_secret_egress_hosts(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(workspace_id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<SecretEgressHost>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SECRET_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        state.store.list_secret_egress_hosts(workspace_id).await?,
    ))
}

/// `DELETE /workspaces/:wid/secret-egress-hosts/:host` — stop trusting a
/// host. The next delivery to it carries the literal refs. `404` when the
/// host was not listed.
pub async fn revoke_secret_egress_host(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((workspace_id, host)): ApiPath<(uuid::Uuid, String)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SECRET_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let host = host.to_ascii_lowercase();
    let revoked = state
        .store
        .revoke_secret_egress_host_audited(
            workspace_id,
            &host,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id),
                action: "secret_egress_host.revoke".into(),
                target_kind: Some("secret_egress_host".into()),
                target_id: None,
                metadata: serde_json::json!({ "workspace_id": workspace_id.0, "host": host }),
            },
        )
        .await?;
    if revoked {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
