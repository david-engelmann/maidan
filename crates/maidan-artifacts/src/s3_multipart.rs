//! S3 multipart upload helpers for large artifacts.

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart as AwsCompletedPart};
use bytes::Bytes;
use uuid::Uuid;

use crate::error::ArtifactError;
use crate::s3::S3Store;
use crate::sha::Sha256;
use crate::store::ArtifactStore;

/// In-progress multipart upload on S3.
#[derive(Debug, Clone)]
pub struct MultipartUpload {
    pub upload_id: String,
    pub object_key: String,
}

/// A part that was uploaded successfully.
#[derive(Debug, Clone)]
pub struct CompletedPart {
    pub part_number: i32,
    pub etag: String,
}

/// A canonical lowercase hyphenated UUID, or nothing. `Uuid::parse_str` accepts
/// uppercase and unhyphenated forms; requiring `to_string() == seg` rejects
/// those, plus `..`, slashes, and any content-addressed key segment.
fn canonical_uuid(seg: &str) -> Option<Uuid> {
    let id = Uuid::parse_str(seg).ok()?;
    (id.to_string() == seg).then_some(id)
}

fn invalid(msg: impl Into<String>) -> ArtifactError {
    ArtifactError::InvalidInput(msg.into())
}

impl MultipartUpload {
    /// Accept only a temporary multipart key and a usable upload id.
    ///
    /// `object_key` is `multipart/<uuid>` (begun with no workspace) or
    /// `multipart/<workspace-uuid>/<uuid>`. Anything else — a content-addressed
    /// blob key, a path with `..`, an extra segment — is refused before S3
    /// sees it.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.upload_id.is_empty()
            || self.upload_id.len() > 1024
            || self.upload_id.trim() != self.upload_id
            || self.upload_id.chars().any(|c| c.is_control())
        {
            return Err(invalid(
                "upload_id must be 1..=1024 bytes, with no leading, trailing, or control characters",
            ));
        }
        let Some(rest) = self.object_key.strip_prefix("multipart/") else {
            return Err(invalid("object_key must be a temporary multipart key"));
        };
        let ok = match rest.split('/').collect::<Vec<_>>().as_slice() {
            [id] => canonical_uuid(id).is_some(),
            [ws, id] => canonical_uuid(ws).is_some() && canonical_uuid(id).is_some(),
            _ => false,
        };
        if !ok {
            return Err(invalid(
                "object_key must be multipart/<uuid> or multipart/<workspace-uuid>/<uuid>",
            ));
        }
        Ok(())
    }

    /// [`Self::validate`], then require the key to belong to `workspace_id`.
    ///
    /// `Some(ws)` accepts only `multipart/{ws}/<uuid>`. An un-namespaced key
    /// and another workspace's key are both refused, so a caller cannot probe,
    /// complete, or abort someone else's upload. `None` is a library caller with
    /// no workspace and accepts either shape.
    pub fn validate_for_workspace(&self, workspace_id: Option<Uuid>) -> Result<(), ArtifactError> {
        self.validate()?;
        let Some(ws) = workspace_id else {
            return Ok(());
        };
        let Some(rest) = self.object_key.strip_prefix("multipart/") else {
            return Err(invalid("object_key must be a temporary multipart key"));
        };
        match rest.split('/').collect::<Vec<_>>().as_slice() {
            [seg, id] if canonical_uuid(seg) == Some(ws) && canonical_uuid(id).is_some() => Ok(()),
            _ => Err(invalid(
                "object_key is not a multipart upload for this workspace",
            )),
        }
    }
}

fn validate_part_number(part_number: i32) -> Result<(), ArtifactError> {
    if !(1..=10_000).contains(&part_number) {
        return Err(invalid("part_number must be between 1 and 10000"));
    }
    Ok(())
}

/// Sort parts by number and refuse an empty list, a bad number, a blank etag,
/// or a repeated part number.
fn validate_completed_parts(parts: &[CompletedPart]) -> Result<Vec<CompletedPart>, ArtifactError> {
    if parts.is_empty() {
        return Err(invalid("at least one part is required"));
    }
    for part in parts {
        validate_part_number(part.part_number)?;
        if part.etag.trim().is_empty() {
            return Err(invalid("etag must not be empty"));
        }
    }
    let mut sorted = parts.to_vec();
    sorted.sort_by_key(|p| p.part_number);
    if sorted
        .windows(2)
        .any(|w| w[0].part_number == w[1].part_number)
    {
        return Err(invalid("duplicate part_number"));
    }
    Ok(sorted)
}

impl S3Store {
    /// Start a multipart upload at an un-namespaced temporary key.
    pub async fn begin_multipart_upload(&self) -> Result<MultipartUpload, ArtifactError> {
        self.begin_multipart_upload_scoped(None).await
    }

    /// Start a multipart upload. `Some(workspace)` nests the temporary key
    /// under that workspace so another tenant cannot name it.
    pub async fn begin_multipart_upload_scoped(
        &self,
        workspace_id: Option<Uuid>,
    ) -> Result<MultipartUpload, ArtifactError> {
        let id = Uuid::now_v7();
        let object_key = match workspace_id {
            Some(ws) => format!("multipart/{ws}/{id}"),
            None => format!("multipart/{id}"),
        };
        let out = self
            .client()
            .create_multipart_upload()
            .bucket(self.bucket())
            .key(&object_key)
            .send()
            .await
            .map_err(|e| ArtifactError::sdk("create_multipart_upload", &e))?;
        let upload_id = out
            .upload_id()
            .ok_or_else(|| ArtifactError::Storage("missing upload_id".into()))?
            .to_string();
        Ok(MultipartUpload {
            upload_id,
            object_key,
        })
    }

    /// Upload one part; returns the ETag required for completion.
    pub async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: i32,
        body: Bytes,
    ) -> Result<String, ArtifactError> {
        upload.validate()?;
        validate_part_number(part_number)?;
        if body.is_empty() {
            return Err(invalid("empty part body"));
        }
        let out = self
            .client()
            .upload_part()
            .bucket(self.bucket())
            .key(&upload.object_key)
            .upload_id(&upload.upload_id)
            .part_number(part_number)
            .body(ByteStream::from(body))
            .send()
            .await
            .map_err(|e| ArtifactError::sdk("upload_part", &e))?;
        out.e_tag()
            .map(|s| s.to_string())
            .ok_or_else(|| ArtifactError::Storage("missing part ETag".into()))
    }

    /// Complete the upload, read back bytes, delete the temporary key, content-address via [`ArtifactStore::put`].
    pub async fn complete_multipart_upload(
        &self,
        upload: &MultipartUpload,
        parts: &[CompletedPart],
    ) -> Result<Sha256, ArtifactError> {
        upload.validate()?;
        let sorted = validate_completed_parts(parts)?;
        let aws_parts: Vec<AwsCompletedPart> = sorted
            .iter()
            .map(|p| {
                AwsCompletedPart::builder()
                    .part_number(p.part_number)
                    .e_tag(&p.etag)
                    .build()
            })
            .collect();
        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(aws_parts))
            .build();
        self.client()
            .complete_multipart_upload()
            .bucket(self.bucket())
            .key(&upload.object_key)
            .upload_id(&upload.upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(|e| ArtifactError::sdk("complete_multipart_upload", &e))?;

        let bytes = match self.get_object_key(&upload.object_key).await {
            Ok(bytes) => bytes,
            Err(err) => {
                let _ = self
                    .client()
                    .delete_object()
                    .bucket(self.bucket())
                    .key(&upload.object_key)
                    .send()
                    .await;
                return Err(err);
            }
        };
        self.client()
            .delete_object()
            .bucket(self.bucket())
            .key(&upload.object_key)
            .send()
            .await
            .map_err(|e| ArtifactError::sdk("delete temp multipart object", &e))?;
        self.put(bytes).await
    }

    /// Abort and best-effort delete a failed multipart upload.
    ///
    /// [`MultipartUpload::validate`] has already required a temporary
    /// `multipart/...` key, so this delete cannot reach a content-addressed blob.
    pub async fn abort_multipart_upload(
        &self,
        upload: &MultipartUpload,
    ) -> Result<(), ArtifactError> {
        upload.validate()?;
        let _ = self
            .client()
            .abort_multipart_upload()
            .bucket(self.bucket())
            .key(&upload.object_key)
            .upload_id(&upload.upload_id)
            .send()
            .await;
        let _ = self
            .client()
            .delete_object()
            .bucket(self.bucket())
            .key(&upload.object_key)
            .send()
            .await;
        Ok(())
    }

    async fn get_object_key(&self, key: &str) -> Result<Bytes, ArtifactError> {
        let out = self
            .client()
            .get_object()
            .bucket(self.bucket())
            .key(key)
            .send()
            .await
            .map_err(|e| ArtifactError::sdk("get_object", &e))?;
        let aggregated = out
            .body
            .collect()
            .await
            .map_err(|e| ArtifactError::sdk("read body", &e))?;
        Ok(aggregated.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(upload_id: &str, object_key: &str) -> MultipartUpload {
        MultipartUpload {
            upload_id: upload_id.to_string(),
            object_key: object_key.to_string(),
        }
    }

    fn scoped_key(ws: Uuid) -> String {
        format!("multipart/{ws}/{}", Uuid::now_v7())
    }

    #[test]
    fn validate_accepts_scoped_and_unscoped_keys() {
        let ws = Uuid::now_v7();
        upload("up-1", &format!("multipart/{}", Uuid::now_v7()))
            .validate()
            .unwrap();
        let key = scoped_key(ws);
        let scoped = upload("up-1", &key);
        scoped.validate().unwrap();
        scoped.validate_for_workspace(Some(ws)).unwrap();
        scoped.validate_for_workspace(None).unwrap();
    }

    #[test]
    fn validate_rejects_content_addressed_and_traversal_keys() {
        let blob = "e3/b0/c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(upload("up-1", blob).validate().is_err());
        let id = Uuid::now_v7();
        assert!(upload("up-1", &format!("multipart/../{id}"))
            .validate()
            .is_err());
        assert!(upload("up-1", &format!("multipart/{id}/extra"))
            .validate()
            .is_err());
        assert!(upload(
            "up-1",
            &format!("multipart/{}", id.to_string().to_ascii_uppercase())
        )
        .validate()
        .is_err());
    }

    #[test]
    fn validate_for_workspace_rejects_other_and_unscoped_keys() {
        let ws_a = Uuid::now_v7();
        let ws_b = Uuid::now_v7();
        let key = scoped_key(ws_a);
        let scoped = upload("up-1", &key);
        assert!(scoped.validate_for_workspace(Some(ws_b)).is_err());
        let unscoped = upload("up-1", &format!("multipart/{}", Uuid::now_v7()));
        unscoped.validate().unwrap();
        assert!(unscoped.validate_for_workspace(Some(ws_a)).is_err());
    }

    #[test]
    fn validate_rejects_bad_upload_ids() {
        let key = format!("multipart/{}", Uuid::now_v7());
        assert!(upload("", &key).validate().is_err());
        assert!(upload(" leading", &key).validate().is_err());
        assert!(upload("trail ", &key).validate().is_err());
        assert!(upload("has\nbreak", &key).validate().is_err());
        assert!(upload(&"x".repeat(1025), &key).validate().is_err());
        upload(&"x".repeat(1024), &key).validate().unwrap();
    }

    #[test]
    fn completed_parts_reject_range_etag_and_duplicates() {
        let bad_numbers = [0, -1, 10_001];
        for n in bad_numbers {
            let err = validate_completed_parts(&[CompletedPart {
                part_number: n,
                etag: "etag".into(),
            }]);
            assert!(err.is_err(), "{n}");
            assert!(validate_part_number(n).is_err());
        }
        assert!(validate_completed_parts(&[CompletedPart {
            part_number: 1,
            etag: "  ".into(),
        }])
        .is_err());
        assert!(validate_completed_parts(&[
            CompletedPart {
                part_number: 2,
                etag: "a".into(),
            },
            CompletedPart {
                part_number: 2,
                etag: "b".into(),
            },
        ])
        .is_err());
        let sorted = validate_completed_parts(&[
            CompletedPart {
                part_number: 2,
                etag: "b".into(),
            },
            CompletedPart {
                part_number: 1,
                etag: "a".into(),
            },
        ])
        .unwrap();
        assert_eq!(sorted[0].part_number, 1);
        assert!(validate_part_number(1).is_ok());
        assert!(validate_part_number(10_000).is_ok());
    }
}
