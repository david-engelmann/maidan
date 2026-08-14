//! Artifact handlers: single-shot and multipart upload, blob fetch, and
//! metadata lookup. The `s3_artifacts`/`multipart_upload` helpers are only
//! used here.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use maidan_artifacts::{ArtifactStore, CompletedPart, MultipartUpload, S3Store, Sha256};
use maidan_auth::{
    capability::{ARTIFACT_UPLOAD, WORKSPACE_READ},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
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
    Path((upload_id, part_number)): Path<(String, i32)>,
    Query(q): Query<MultipartUploadQuery>,
    body: Bytes,
) -> ApiResult<Json<MultipartPartResponse>> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty part body".into()));
    }
    let upload = multipart_upload(&upload_id, &q.object_key);
    let etag = s3_artifacts(&state.artifacts)?
        .upload_part(&upload, part_number, body)
        .await?;
    Ok(Json(MultipartPartResponse { part_number, etag }))
}

pub async fn complete_multipart_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(upload_id): Path<String>,
    ApiJson(body): ApiJson<CompleteMultipartArtifact>,
) -> ApiResult<(StatusCode, Json<Artifact>)> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    let upload = multipart_upload(&upload_id, &body.object_key);
    let parts: Vec<CompletedPart> = body
        .parts
        .into_iter()
        .map(|p| CompletedPart {
            part_number: p.part_number,
            etag: p.etag,
        })
        .collect();
    let sha = s3_artifacts(&state.artifacts)?
        .complete_multipart_upload(&upload, &parts)
        .await?;
    let bytes = state.artifacts.get(&sha).await?;
    // Cluster 214: upsert + the Cluster-204 per-workspace ref + the ArtifactUpserted
    // event commit atomically (ref only for a non-bypass caller, as before).
    let ref_workspace = (!auth.bypass).then_some(auth.workspace_id);
    let (artifact, stored) = state
        .store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha.to_string(),
                size_bytes: bytes.len() as i64,
                mime_type: body.mime_type,
                kind: body.kind,
                uploaded_by: body.uploaded_by.map(MemberId),
            },
            ref_workspace,
        )
        .await?;
    super::publish_stored(&state, stored).await;
    Ok((StatusCode::CREATED, Json(artifact)))
}

pub async fn abort_multipart_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<AbortMultipartQuery>,
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
    Query(q): Query<UploadArtifactQuery>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Artifact>)> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty artifact body".into()));
    }
    let sha = state.artifacts.put(body.clone()).await?;
    // Cluster 214: upsert + the Cluster-204 per-workspace ref (so only the
    // uploader's workspace can fetch the deduped blob) + the ArtifactUpserted event
    // commit atomically; the ref is recorded only for a non-bypass caller.
    let ref_workspace = (!auth.bypass).then_some(auth.workspace_id);
    let (artifact, stored) = state
        .store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha.to_string(),
                size_bytes: body.len() as i64,
                mime_type: q.mime_type,
                kind: q.kind,
                uploaded_by: q.uploaded_by.map(MemberId),
            },
            ref_workspace,
        )
        .await?;
    super::publish_stored(&state, stored).await;
    Ok((StatusCode::CREATED, Json(artifact)))
}

/// Cluster 204: 404 (not 403) when the caller's workspace has no access link to
/// `sha` — a missing link and a missing artifact are indistinguishable to the
/// caller, so a cross-tenant SHA can't be confirmed to exist.
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
    Path(sha_hex): Path<String>,
) -> ApiResult<Response> {
    cap(&auth, WORKSPACE_READ)?;
    let sha = Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    ensure_artifact_ref(&state, &auth, &sha_hex).await?;
    let meta = state.store.get_artifact_by_sha(&sha_hex).await?;
    let bytes = state.artifacts.get(&sha).await?;
    let mut headers = HeaderMap::new();
    if let Some(mime) = meta.mime_type {
        if let Ok(value) = mime.parse() {
            headers.insert(header::CONTENT_TYPE, value);
        }
    }
    if let Ok(kind) = meta.kind.as_str().parse() {
        headers.insert(header::HeaderName::from_static("x-artifact-kind"), kind);
    }
    Ok((headers, bytes).into_response())
}

pub async fn get_artifact_metadata(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(sha_hex): Path<String>,
) -> ApiResult<Json<Artifact>> {
    cap(&auth, WORKSPACE_READ)?;
    Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    ensure_artifact_ref(&state, &auth, &sha_hex).await?;
    Ok(Json(state.store.get_artifact_by_sha(&sha_hex).await?))
}
