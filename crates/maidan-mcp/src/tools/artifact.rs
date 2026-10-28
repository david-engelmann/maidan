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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadArtifactArgs {
    kind: ArtifactKind,
    content_base64: String,
    mime_type: Option<String>,
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
    server: &crate::server::McpServer,
    auth: &AuthContext,
) -> Result<Value, McpError> {
    refuse_stored_bytes(server, auth)?;
    let upload = s3_artifacts(&server.artifacts)?
        .begin_multipart_upload()
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    Ok(json!({
        "upload_id": upload.upload_id,
        "object_key": upload.object_key,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadMultipartPartArgs {
    upload_id: String,
    object_key: String,
    part_number: i32,
    content_base64: String,
}

pub(super) async fn upload_artifact_multipart_part(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UploadMultipartPartArgs = crate::tools::parse_args(args)?;
    refuse_stored_bytes(server, auth)?;
    let raw = STANDARD
        .decode(&a.content_base64)
        .map_err(|e| McpError::InvalidParams(format!("invalid base64: {e}")))?;
    let upload = multipart_upload(&a.upload_id, &a.object_key);
    let etag = s3_artifacts(&server.artifacts)?
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
#[serde(deny_unknown_fields)]
struct CompleteMultipartArgs {
    upload_id: String,
    object_key: String,
    parts: Vec<MultipartPartArg>,
    kind: ArtifactKind,
    mime_type: Option<String>,
}

pub(super) async fn complete_artifact_multipart(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CompleteMultipartArgs = crate::tools::parse_args(args)?;
    let upload = multipart_upload(&a.upload_id, &a.object_key);
    let parts: Vec<CompletedPart> = a
        .parts
        .into_iter()
        .map(|p| CompletedPart {
            part_number: p.part_number,
            etag: p.etag,
        })
        .collect();
    // Cut off before completing: drop the parts, so a refusal stores nothing.
    if let Err(refused) = refuse_stored_bytes(server, auth) {
        if let Err(err) = s3_artifacts(&server.artifacts)?
            .abort_multipart_upload(&upload)
            .await
        {
            tracing::warn!(error = %err, "artifact.refused_parts_not_aborted");
        }
        return Err(refused);
    }
    let sha = s3_artifacts(&server.artifacts)?
        .complete_multipart_upload(&upload, &parts)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    let bytes = server
        .artifacts
        .get(&sha)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    // The parts are already stored, so the size is known only now. A refused
    // upload's bytes go unless another workspace holds them.
    if let Err(refused) = ensure_media_quota(server, auth, a.kind, &sha.to_hex(), bytes.len()).await
    {
        reap_if_orphaned(server, auth.workspace_id, &sha).await;
        return Err(refused);
    }
    let (artifact, stored) = upsert_uploaded_artifact(
        server,
        auth,
        NewArtifact {
            sha256: sha.to_string(),
            size_bytes: bytes.len() as i64,
            mime_type: a.mime_type,
            filename: None,
            kind: a.kind,
            uploaded_by: (!auth.bypass).then_some(auth.member_id),
        },
    )
    .await?;
    // A last-reference erase may have reaped the bytes between the put and the row.
    maidan_artifacts::restore_if_reaped(server.artifacts.as_ref(), &sha, bytes)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    server.publish_stored(&stored).await;
    Ok(content_json(&artifact))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AbortMultipartArgs {
    upload_id: String,
    object_key: String,
}

pub(super) async fn abort_artifact_multipart(
    artifacts: &Arc<dyn ArtifactStore>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AbortMultipartArgs = crate::tools::parse_args(args)?;
    let upload = multipart_upload(&a.upload_id, &a.object_key);
    s3_artifacts(artifacts)?
        .abort_multipart_upload(&upload)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    Ok(json!({ "aborted": true }))
}

pub(super) async fn upload_artifact(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UploadArtifactArgs = crate::tools::parse_args(args)?;
    let raw = STANDARD
        .decode(&a.content_base64)
        .map_err(|e| McpError::InvalidParams(format!("invalid base64: {e}")))?;
    let bytes = Bytes::from(raw);
    // Refused before the bytes are written, so a refusal stores nothing: the
    // stored-bytes cutoff and the media quota are both checked first.
    refuse_stored_bytes(server, auth)?;
    let upload_sha = maidan_artifacts::Sha256::compute(&bytes);
    ensure_media_quota(server, auth, a.kind, &upload_sha.to_hex(), bytes.len()).await?;
    let sha = server
        .artifacts
        .put(bytes.clone())
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    let (artifact, stored) = upsert_uploaded_artifact(
        server,
        auth,
        NewArtifact {
            sha256: sha.to_string(),
            size_bytes: bytes.len() as i64,
            mime_type: a.mime_type,
            filename: None,
            kind: a.kind,
            uploaded_by: (!auth.bypass).then_some(auth.member_id),
        },
    )
    .await?;
    // A last-reference erase may have reaped the bytes between the put and the row.
    maidan_artifacts::restore_if_reaped(server.artifacts.as_ref(), &sha, bytes)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    server.publish_stored(&stored).await;
    Ok(content_json(&artifact))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetArtifactMetadataArgs {
    #[serde(deserialize_with = "sha256_hex")]
    sha256: String,
}

/// A sha256 hex digest in its one stored form, lowercase, refused while
/// parsing so a malformed one says so instead of reading as an artifact that
/// does not exist.
pub(super) fn sha256_hex<'de, D: serde::Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    let hex = String::deserialize(de)?;
    maidan_artifacts::Sha256::from_hex(&hex)
        .map(|sha| sha.to_hex())
        .map_err(serde::de::Error::custom)
}

pub(super) async fn get_artifact_metadata(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetArtifactMetadataArgs = crate::tools::parse_args(args)?;
    // A blob is deduped across tenants, so gate on the caller's per-workspace
    // access ref. A missing ref returns NotFound (indistinguishable from a
    // genuinely-absent artifact — no cross-tenant existence oracle), exactly as
    // the REST `get_artifact` does.
    if !auth.bypass
        && !store
            .artifact_ref_exists(auth.workspace_id, &a.sha256)
            .await?
    {
        return Err(McpError::NotFound);
    }
    let artifact = if auth.bypass {
        store.get_artifact_by_sha(&a.sha256).await?
    } else {
        store
            .get_artifact_for_workspace(auth.workspace_id, &a.sha256)
            .await?
    };
    Ok(content_json(&artifact))
}

/// Artifact bytes plus the caller workspace metadata. Twin of `GET /artifacts/{sha}`:
/// the access ref is checked first (missing ref is `NotFound`, not a cross-tenant
/// oracle), then the blob is loaded. `content_base64` is the raw bytes.
pub(super) async fn get_artifact(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetArtifactMetadataArgs = crate::tools::parse_args(args)?;
    let sha = maidan_artifacts::Sha256::from_hex(&a.sha256)
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    if !auth.bypass
        && !server
            .store
            .artifact_ref_exists(auth.workspace_id, &a.sha256)
            .await?
    {
        return Err(McpError::NotFound);
    }
    let artifact = if auth.bypass {
        server.store.get_artifact_by_sha(&a.sha256).await?
    } else {
        server
            .store
            .get_artifact_for_workspace(auth.workspace_id, &a.sha256)
            .await?
    };
    let bytes = server
        .artifacts
        .get(&sha)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    Ok(content_json(&json!({
        "artifact": artifact,
        "content_base64": STANDARD.encode(bytes),
    })))
}

/// The media quota, as the REST upload routes apply it: a media upload that
/// would take the caller's workspace past its quota is refused, naming the
/// workspace's usage, its quota and the upload's size. An unscoped (bypass)
/// upload records no workspace ref, so no quota applies.
async fn ensure_media_quota(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    kind: ArtifactKind,
    sha256: &str,
    upload_bytes: usize,
) -> Result<(), McpError> {
    if auth.bypass {
        return Ok(());
    }
    let upload_bytes = i64::try_from(upload_bytes).unwrap_or(i64::MAX);
    match maidan_store::media_retention::check_media_quota(
        server.store.as_ref(),
        auth.workspace_id,
        &server.instance_retention(),
        kind,
        sha256,
        upload_bytes,
    )
    .await?
    {
        Some(refused) => Err(McpError::InvalidParams(refused.to_string())),
        None => Ok(()),
    }
}

/// `OverAllowance` when the caller's workspace is cut off on `stored_bytes`.
/// Upload tools call this before any bytes are written, so a refused upload
/// stores nothing. An unscoped (bypass) upload is not metered.
pub(super) fn refuse_stored_bytes(
    server: &crate::server::McpServer,
    auth: &AuthContext,
) -> Result<(), McpError> {
    if auth.bypass {
        return Ok(());
    }
    server
        .store
        .usage_meters()
        .refuse(auth.workspace_id, UsageMeter::StoredBytes)
        .map_err(McpError::from)
}

/// The artifact upsert every upload tool ends in. When the store refuses it
/// with `OverAllowance` (the workspace was cut off after the pre-check, before
/// the row), the bytes just put go too, unless an artifact row, in any
/// workspace, already holds that sha.
pub(super) async fn upsert_uploaded_artifact(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    new: NewArtifact,
) -> Result<(Artifact, StoredEvent), McpError> {
    let ref_workspace = (!auth.bypass).then_some(auth.workspace_id);
    let sha = maidan_artifacts::Sha256::from_hex(&new.sha256).ok();
    match server
        .store
        .upsert_artifact_with_event(new, ref_workspace)
        .await
    {
        Err(refused @ maidan_store::StoreError::OverAllowance(_)) => {
            if let Some(sha) = sha {
                reap_if_orphaned(server, auth.workspace_id, &sha).await;
            }
            Err(refused.into())
        }
        other => Ok(other?),
    }
}

/// Delete the bytes of a refused upload unless an artifact row
/// holds them, under the reap lease every erase uses. A failed delete is
/// queued for the retention sweep to retry, and logged, not returned: the
/// bytes are only unreachable.
async fn reap_if_orphaned(
    server: &crate::server::McpServer,
    workspace_id: maidan_types::WorkspaceId,
    sha: &maidan_artifacts::Sha256,
) {
    let artifacts = server.artifacts.clone();
    let target = *sha;
    let delete: maidan_store::BlobDelete<'_> = Box::new(move || {
        Box::pin(async move {
            match artifacts.delete(&target).await {
                Ok(()) | Err(maidan_artifacts::ArtifactError::NotFound) => Ok(()),
                Err(err) => Err(err.to_string()),
            }
        })
    });
    let reaped = maidan_store::blob_reap_retry::reap_or_queue(
        server.store.as_ref(),
        &sha.to_hex(),
        workspace_id,
        delete,
    )
    .await;
    if let maidan_store::BlobReap::DeleteFailed(err) = reaped {
        tracing::warn!(sha = %sha, error = %err, "artifact.refused_blob_not_deleted");
    }
}

#[cfg(test)]
mod stored_bytes_cutoff_tests {
    use super::*;
    use crate::server::McpServer;
    use maidan_artifacts::{LocalFsStore, Sha256};
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore};

    struct Fixture {
        server: McpServer,
        _dir: tempfile::TempDir,
        cut: AuthContext,
        other: AuthContext,
    }

    async fn fixture() -> Fixture {
        let pool = maidan_search::sqlite_pool_options()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
        let dir = tempfile::tempdir().unwrap();
        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(dir.path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let mut auths = Vec::new();
        for name in ["cut", "other"] {
            let ws = store
                .create_workspace(NewWorkspace { name: name.into() })
                .await
                .unwrap();
            let m = store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: name.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap();
            auths.push(AuthContext::from_session(m.id, ws.id, vec![]));
        }
        let other = auths.remove(1);
        let cut = auths.remove(0);
        Fixture {
            server,
            _dir: dir,
            cut,
            other,
        }
    }

    fn cut_off(f: &Fixture) {
        f.server
            .store
            .usage_meters()
            .set_cut_off([(f.cut.workspace_id, UsageMeter::StoredBytes)]);
    }

    fn new_artifact(sha: &Sha256, auth: &AuthContext, len: usize) -> NewArtifact {
        NewArtifact {
            sha256: sha.to_string(),
            size_bytes: len as i64,
            mime_type: None,
            filename: None,
            kind: ArtifactKind::Attachment,
            uploaded_by: Some(auth.member_id),
        }
    }

    #[tokio::test]
    async fn a_cut_off_upload_is_refused_before_any_bytes_are_written() {
        let f = fixture().await;
        cut_off(&f);
        let bytes = b"refused before the put".to_vec();
        let args = json!({ "kind": "attachment", "content_base64": STANDARD.encode(&bytes) });
        let refused = upload_artifact(&f.server, &f.cut, &args).await;
        assert!(
            matches!(
                refused,
                Err(McpError::OverAllowance(UsageMeter::StoredBytes))
            ),
            "{refused:?}"
        );
        let sha = Sha256::compute(&bytes);
        assert!(!f.server.artifacts.exists(&sha).await.unwrap());
        assert!(matches!(
            begin_artifact_multipart(&f.server, &f.cut).await,
            Err(McpError::OverAllowance(UsageMeter::StoredBytes))
        ));
    }

    #[tokio::test]
    async fn a_cutoff_landing_after_the_put_reaps_the_bytes_it_refuses() {
        let f = fixture().await;
        let bytes = Bytes::from_static(b"put, then cut off before the row");
        let sha = f.server.artifacts.put(bytes.clone()).await.unwrap();
        cut_off(&f);
        let refused =
            upsert_uploaded_artifact(&f.server, &f.cut, new_artifact(&sha, &f.cut, bytes.len()))
                .await;
        assert!(
            matches!(
                refused,
                Err(McpError::OverAllowance(UsageMeter::StoredBytes))
            ),
            "{refused:?}"
        );
        assert!(!f.server.artifacts.exists(&sha).await.unwrap());
    }

    #[tokio::test]
    async fn a_refused_upload_keeps_bytes_another_workspace_already_holds() {
        let f = fixture().await;
        let bytes = Bytes::from_static(b"the same bytes in two workspaces");
        let sha = f.server.artifacts.put(bytes.clone()).await.unwrap();
        upsert_uploaded_artifact(
            &f.server,
            &f.other,
            new_artifact(&sha, &f.other, bytes.len()),
        )
        .await
        .unwrap();
        cut_off(&f);
        let refused =
            upsert_uploaded_artifact(&f.server, &f.cut, new_artifact(&sha, &f.cut, bytes.len()))
                .await;
        assert!(refused.is_err());
        assert!(f.server.artifacts.exists(&sha).await.unwrap());
        assert_eq!(
            f.server.artifacts.get(&sha).await.unwrap(),
            bytes,
            "the other workspace's artifact still reads"
        );
    }
}
