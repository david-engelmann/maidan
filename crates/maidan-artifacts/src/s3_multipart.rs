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

impl S3Store {
    /// Start a multipart upload at a temporary object key.
    pub async fn begin_multipart_upload(&self) -> Result<MultipartUpload, ArtifactError> {
        let object_key = format!("multipart/{}", Uuid::new_v4());
        let out = self
            .client()
            .create_multipart_upload()
            .bucket(self.bucket())
            .key(&object_key)
            .send()
            .await
            .map_err(|e| ArtifactError::Storage(format!("create_multipart_upload: {e}")))?;
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
        if part_number < 1 {
            return Err(ArtifactError::InvalidInput(
                "part_number must be >= 1".into(),
            ));
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
            .map_err(|e| ArtifactError::Storage(format!("upload_part: {e}")))?;
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
        if parts.is_empty() {
            return Err(ArtifactError::InvalidInput(
                "at least one part is required".into(),
            ));
        }
        let mut sorted = parts.to_vec();
        sorted.sort_by_key(|p| p.part_number);
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
            .map_err(|e| ArtifactError::Storage(format!("complete_multipart_upload: {e}")))?;

        let bytes = self.get_object_key(&upload.object_key).await?;
        self.client()
            .delete_object()
            .bucket(self.bucket())
            .key(&upload.object_key)
            .send()
            .await
            .map_err(|e| ArtifactError::Storage(format!("delete temp multipart object: {e}")))?;
        self.put(bytes).await
    }

    /// Abort and best-effort delete a failed multipart upload.
    pub async fn abort_multipart_upload(
        &self,
        upload: &MultipartUpload,
    ) -> Result<(), ArtifactError> {
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
            .map_err(|e| ArtifactError::Storage(format!("get_object: {e}")))?;
        let aggregated = out
            .body
            .collect()
            .await
            .map_err(|e| ArtifactError::Storage(format!("read body: {e}")))?;
        Ok(aggregated.into_bytes())
    }
}
