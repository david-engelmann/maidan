//! Artifact upload (single-shot + multipart) and metadata tool handlers.

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, CompletedPart, MultipartUpload, S3Store};
use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

/// Record the per-workspace access ref for an artifact so the caller's workspace
/// can later fetch the deduped blob (Cluster 204 / 332). Skipped for a bypass
/// caller (auth disabled). Mirrors the REST upload path (`ref_workspace`).
async fn record_ref(store: &Arc<dyn Store>, auth: &AuthContext, sha: &str) -> Result<(), McpError> {
    if !auth.bypass {
        store.record_artifact_ref(auth.workspace_id, sha).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct UploadArtifactArgs {
    kind: ArtifactKind,
    content_base64: String,
    mime_type: Option<String>,
    uploaded_by: Option<uuid::Uuid>,
}

fn s3_artifacts(artifacts: &Arc<dyn ArtifactStore>) -> Result<&S3Store, McpError> {
    artifacts
        .as_ref()
        .as_any()
        .downcast_ref::<S3Store>()
        .ok_or_else(|| {
            McpError::InvalidParams(
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

pub(super) async fn begin_artifact_multipart(
    artifacts: &Arc<dyn ArtifactStore>,
) -> Result<Value, McpError> {
    let upload = s3_artifacts(artifacts)?
        .begin_multipart_upload()
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    Ok(json!({
        "upload_id": upload.upload_id,
        "object_key": upload.object_key,
    }))
}

#[derive(Deserialize)]
struct UploadMultipartPartArgs {
    upload_id: String,
    object_key: String,
    part_number: i32,
    content_base64: String,
}

pub(super) async fn upload_artifact_multipart_part(
    artifacts: &Arc<dyn ArtifactStore>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UploadMultipartPartArgs = serde_json::from_value(args.clone())?;
    let raw = STANDARD
        .decode(&a.content_base64)
        .map_err(|e| McpError::InvalidParams(format!("invalid base64: {e}")))?;
    let upload = multipart_upload(&a.upload_id, &a.object_key);
    let etag = s3_artifacts(artifacts)?
        .upload_part(&upload, a.part_number, Bytes::from(raw))
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    Ok(json!({
        "part_number": a.part_number,
        "etag": etag,
    }))
}

#[derive(Deserialize)]
struct MultipartPartArg {
    part_number: i32,
    etag: String,
}

#[derive(Deserialize)]
struct CompleteMultipartArgs {
    upload_id: String,
    object_key: String,
    parts: Vec<MultipartPartArg>,
    kind: ArtifactKind,
    mime_type: Option<String>,
    uploaded_by: Option<uuid::Uuid>,
}

pub(super) async fn complete_artifact_multipart(
    store: &Arc<dyn Store>,
    artifacts: &Arc<dyn ArtifactStore>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CompleteMultipartArgs = serde_json::from_value(args.clone())?;
    let upload = multipart_upload(&a.upload_id, &a.object_key);
    let parts: Vec<CompletedPart> = a
        .parts
        .into_iter()
        .map(|p| CompletedPart {
            part_number: p.part_number,
            etag: p.etag,
        })
        .collect();
    let sha = s3_artifacts(artifacts)?
        .complete_multipart_upload(&upload, &parts)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    let bytes = artifacts
        .get(&sha)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    let artifact = store
        .upsert_artifact(NewArtifact {
            sha256: sha.to_string(),
            size_bytes: bytes.len() as i64,
            mime_type: a.mime_type,
            kind: a.kind,
            uploaded_by: a.uploaded_by.map(MemberId),
        })
        .await?;
    record_ref(store, auth, &sha.to_string()).await?;
    Ok(content_json(&artifact))
}

#[derive(Deserialize)]
struct AbortMultipartArgs {
    upload_id: String,
    object_key: String,
}

pub(super) async fn abort_artifact_multipart(
    artifacts: &Arc<dyn ArtifactStore>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AbortMultipartArgs = serde_json::from_value(args.clone())?;
    let upload = multipart_upload(&a.upload_id, &a.object_key);
    s3_artifacts(artifacts)?
        .abort_multipart_upload(&upload)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    Ok(json!({ "aborted": true }))
}

pub(super) async fn upload_artifact(
    store: &Arc<dyn Store>,
    artifacts: &Arc<dyn ArtifactStore>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UploadArtifactArgs = serde_json::from_value(args.clone())?;
    let raw = STANDARD
        .decode(&a.content_base64)
        .map_err(|e| McpError::InvalidParams(format!("invalid base64: {e}")))?;
    let bytes = Bytes::from(raw);
    let sha = artifacts
        .put(bytes.clone())
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    let artifact = store
        .upsert_artifact(NewArtifact {
            sha256: sha.to_string(),
            size_bytes: bytes.len() as i64,
            mime_type: a.mime_type,
            kind: a.kind,
            uploaded_by: a.uploaded_by.map(MemberId),
        })
        .await?;
    record_ref(store, auth, &sha.to_string()).await?;
    Ok(content_json(&artifact))
}

#[derive(Deserialize)]
struct GetArtifactMetadataArgs {
    sha256: String,
}

pub(super) async fn get_artifact_metadata(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetArtifactMetadataArgs = serde_json::from_value(args.clone())?;
    // Cluster 204/332: a blob is deduped across tenants, so gate on the caller's
    // per-workspace access ref. A missing ref returns NotFound (indistinguishable
    // from a genuinely-absent artifact — no cross-tenant existence oracle), exactly
    // as the REST `get_artifact` does.
    if !auth.bypass
        && !store
            .artifact_ref_exists(auth.workspace_id, &a.sha256)
            .await?
    {
        return Err(McpError::NotFound);
    }
    let artifact = store.get_artifact_by_sha(&a.sha256).await?;
    Ok(content_json(&artifact))
}
