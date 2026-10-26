//! Artifact handlers: single-shot and multipart upload, blob fetch, and
//! metadata lookup. The `s3_artifacts`/`multipart_upload` helpers are only
//! used here.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::Response, Extension, Json};
use maidan_artifacts::{ArtifactStore, CompletedPart, MultipartUpload, S3Store, Sha256};
use maidan_auth::{
    capability::{ARTIFACT_UPLOAD, TOKEN_ADMIN, WORKSPACE_READ},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ApiResult};
use crate::artifact_response::upload_filename;
use crate::dto::*;
use crate::error::ApiError;
use crate::extract::{ApiBytes, ApiJson, ApiPath, ApiQuery};
use crate::state::AppState;

fn s3_artifacts(artifacts: &Arc<dyn ArtifactStore>) -> Result<&S3Store, ApiError> {
    artifacts
        .as_ref()
        .as_any()
        .downcast_ref::<S3Store>()
        .ok_or_else(|| {
            ApiError::BadRequest(
                "multipart uploads require S3 artifact backend (ARTIFACT_BACKEND=s3)".into(),
            )
        })
}

fn multipart_upload(upload_id: &str, object_key: &str) -> MultipartUpload {
    MultipartUpload {
        upload_id: upload_id.to_string(),
        object_key: object_key.to_string(),
    }
}

pub async fn begin_multipart_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<(StatusCode, Json<MultipartUploadResponse>)> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    super::refuse_stored_bytes(&state, &auth)?;
    let upload = s3_artifacts(&state.artifacts)?
        .begin_multipart_upload()
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(MultipartUploadResponse {
            upload_id: upload.upload_id,
            object_key: upload.object_key,
        }),
    ))
}

pub async fn upload_multipart_artifact_part(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((upload_id, part_number)): ApiPath<(String, i32)>,
    ApiQuery(q): ApiQuery<MultipartUploadQuery>,
    ApiBytes(body): ApiBytes,
) -> ApiResult<Json<MultipartPartResponse>> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty part body".into()));
    }
    super::refuse_stored_bytes(&state, &auth)?;
    let upload = multipart_upload(&upload_id, &q.object_key);
    let etag = s3_artifacts(&state.artifacts)?
        .upload_part(&upload, part_number, body)
        .await?;
    Ok(Json(MultipartPartResponse { part_number, etag }))
}

pub async fn complete_multipart_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(upload_id): ApiPath<String>,
    ApiJson(body): ApiJson<CompleteMultipartArtifact>,
) -> ApiResult<(StatusCode, Json<Artifact>)> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    let filename = upload_filename(body.filename)?;
    let upload = multipart_upload(&upload_id, &body.object_key);
    let parts: Vec<CompletedPart> = body
        .parts
        .into_iter()
        .map(|p| CompletedPart {
            part_number: p.part_number,
            etag: p.etag,
        })
        .collect();
    // Cut off before completing: drop the parts, so a refusal stores nothing.
    if let Err(refused) = super::refuse_stored_bytes(&state, &auth) {
        if let Err(err) = s3_artifacts(&state.artifacts)?
            .abort_multipart_upload(&upload)
            .await
        {
            tracing::warn!(error = %err, "artifact.refused_parts_not_aborted");
        }
        return Err(refused);
    }
    let sha = s3_artifacts(&state.artifacts)?
        .complete_multipart_upload(&upload, &parts)
        .await?;
    let bytes = state.artifacts.get(&sha).await?;
    // The parts are already stored, so the size is known only now. A refused
    // upload's bytes go unless another workspace holds them.
    if let Err(refused) =
        ensure_media_quota(&state, &auth, body.kind, &sha.to_hex(), bytes.len() as i64).await
    {
        super::reap_orphaned_blobs(
            state.store.as_ref(),
            &state.artifacts,
            auth.workspace_id,
            &[sha.to_hex()],
        )
        .await;
        return Err(refused);
    }
    // Upsert + the per-workspace ref + the ArtifactUpserted event commit
    // atomically (ref only for a non-bypass caller). A cutoff that lands
    // after the pre-check is refused here and the bytes are reaped.
    let (artifact, stored) = super::upsert_uploaded_artifact(
        &state,
        &auth,
        NewArtifact {
            sha256: sha.to_string(),
            size_bytes: bytes.len() as i64,
            mime_type: body.mime_type,
            filename,
            kind: body.kind,
            uploaded_by: (!auth.bypass).then_some(auth.member_id),
        },
    )
    .await?;
    // A last-reference erase may have reaped the bytes between the put and the row.
    maidan_artifacts::restore_if_reaped(state.artifacts.as_ref(), &sha, bytes).await?;
    super::publish_stored(&state, stored).await;
    Ok((StatusCode::CREATED, Json(artifact)))
}

pub async fn abort_multipart_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiQuery(q): ApiQuery<AbortMultipartQuery>,
) -> ApiResult<StatusCode> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    let upload = multipart_upload(&q.upload_id, &q.object_key);
    s3_artifacts(&state.artifacts)?
        .abort_multipart_upload(&upload)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn upload_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiQuery(q): ApiQuery<UploadArtifactQuery>,
    ApiBytes(body): ApiBytes,
) -> ApiResult<(StatusCode, Json<Artifact>)> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty artifact body".into()));
    }
    let filename = upload_filename(q.filename)?;
    // Refused before the bytes are written, so a refusal stores nothing: the
    // stored-bytes cutoff and the media quota are both checked first.
    super::refuse_stored_bytes(&state, &auth)?;
    let upload_sha = Sha256::compute(&body);
    ensure_media_quota(
        &state,
        &auth,
        q.kind,
        &upload_sha.to_hex(),
        body.len() as i64,
    )
    .await?;
    let sha = state.artifacts.put(body.clone()).await?;
    // Upsert + the per-workspace ref + the ArtifactUpserted event commit
    // atomically (ref only for a non-bypass caller). A cutoff that lands
    // after the pre-check is refused here and the bytes are reaped.
    let (artifact, stored) = super::upsert_uploaded_artifact(
        &state,
        &auth,
        NewArtifact {
            sha256: sha.to_string(),
            size_bytes: body.len() as i64,
            mime_type: q.mime_type,
            filename,
            kind: q.kind,
            uploaded_by: (!auth.bypass).then_some(auth.member_id),
        },
    )
    .await?;
    // A last-reference erase may have reaped the bytes between the put and the row.
    maidan_artifacts::restore_if_reaped(state.artifacts.as_ref(), &sha, body).await?;
    super::publish_stored(&state, stored).await;
    Ok((StatusCode::CREATED, Json(artifact)))
}

/// 409 `media-quota-exceeded` when this media upload would take the caller's
/// workspace past its media quota. An unscoped (bypass) upload records no
/// workspace ref, so no workspace's quota applies to it.
async fn ensure_media_quota(
    state: &AppState,
    auth: &AuthContext,
    kind: ArtifactKind,
    sha256: &str,
    upload_bytes: i64,
) -> ApiResult<()> {
    if auth.bypass {
        return Ok(());
    }
    match maidan_store::media_retention::check_media_quota(
        state.store.as_ref(),
        auth.workspace_id,
        &state.mcp.instance_retention(),
        kind,
        sha256,
        upload_bytes,
    )
    .await?
    {
        Some(refused) => Err(refused.into()),
        None => Ok(()),
    }
}

/// 404 (not 403) when the caller's workspace has no access link to `sha` — a
/// missing link and a missing artifact are indistinguishable to the caller, so
/// a cross-tenant SHA can't be confirmed to exist.
async fn ensure_artifact_ref(state: &AppState, auth: &AuthContext, sha256: &str) -> ApiResult<()> {
    if auth.bypass {
        return Ok(());
    }
    if state
        .store
        .artifact_ref_exists(auth.workspace_id, sha256)
        .await?
    {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn get_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(sha_hex): ApiPath<String>,
) -> ApiResult<Response> {
    cap(&auth, WORKSPACE_READ)?;
    let sha = Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    ensure_artifact_ref(&state, &auth, &sha_hex).await?;
    let meta = artifact_as_seen_by(&state, &auth, &sha_hex).await?;
    let bytes = state.artifacts.get(&sha).await?;
    Ok(crate::artifact_response::artifact_response(&meta, bytes))
}

pub async fn get_artifact_metadata(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(sha_hex): ApiPath<String>,
) -> ApiResult<Json<Artifact>> {
    cap(&auth, WORKSPACE_READ)?;
    Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    ensure_artifact_ref(&state, &auth, &sha_hex).await?;
    Ok(Json(artifact_as_seen_by(&state, &auth, &sha_hex).await?))
}

/// `DELETE /artifacts/:sha` — erase the caller's workspace's copy of an
/// artifact. `token:admin`, audited as `artifact.erase`, refused (409) under a
/// legal hold. Artifacts are deduplicated across workspaces: the workspace
/// loses access at once, and the bytes are deleted only when no other
/// workspace references them.
pub async fn erase_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(sha_hex): ApiPath<String>,
) -> ApiResult<Json<ArtifactErasure>> {
    cap(&auth, TOKEN_ADMIN)?;
    let sha = Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let actor = auth.actor_id;
    let erasure = state
        .store
        .erase_artifact_audited(
            auth.workspace_id,
            &sha.to_hex(),
            Box::new(move |erasure| NewAuditEvent {
                scope: AuditScope::Workspace(erasure.workspace_id),
                actor_id: Some(actor),
                action: "artifact.erase".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(erasure.workspace_id.0),
                metadata: serde_json::json!({
                    "sha256": erasure.sha256,
                    "last_reference": erasure.last_reference,
                }),
            }),
        )
        .await?;
    if erasure.last_reference {
        super::workspace::delete_orphaned_blobs(
            &state,
            erasure.workspace_id,
            std::slice::from_ref(&erasure.sha256),
        )
        .await;
    }
    Ok(Json(erasure))
}

/// The artifact with the caller's workspace's own metadata. The shared row's
/// `uploaded_by` is whoever uploaded the bytes first, possibly in another
/// tenant. A workspace that erased its reference gets `NotFound`.
async fn artifact_as_seen_by(
    state: &AppState,
    auth: &AuthContext,
    sha_hex: &str,
) -> ApiResult<Artifact> {
    let artifact = if auth.bypass {
        state.store.get_artifact_by_sha(sha_hex).await?
    } else {
        state
            .store
            .get_artifact_for_workspace(auth.workspace_id, sha_hex)
            .await?
    };
    Ok(artifact)
}
