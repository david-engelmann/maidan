//! S3-compatible implementation of [`ArtifactStore`] (MinIO, AWS S3).

use std::sync::Arc;

use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use bytes::Bytes;
use tracing::instrument;

use crate::error::ArtifactError;
use crate::path::object_key;
use crate::sha::Sha256;
use crate::store::ArtifactStore;

/// Connection parameters for [`S3Store`].
#[derive(Debug, Clone)]
pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

#[derive(Debug, Clone)]
pub struct S3Store {
    client: Client,
    bucket: String,
}

impl S3Store {
    pub async fn new(config: S3Config) -> Result<Self, ArtifactError> {
        let creds = Credentials::new(
            config.access_key,
            config.secret_key,
            None,
            None,
            "maidan-artifacts",
        );
        let shared = aws_config::defaults(BehaviorVersion::latest())
            .endpoint_url(&config.endpoint)
            .region(Region::new(config.region.clone()))
            .credentials_provider(creds)
            .load()
            .await;
        let s3_conf = aws_sdk_s3::config::Builder::from(&shared)
            .force_path_style(true)
            .build();
        let client = Client::from_conf(s3_conf);
        let store = Self {
            client,
            bucket: config.bucket,
        };
        store.ensure_bucket().await?;
        Ok(store)
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    async fn ensure_bucket(&self) -> Result<(), ArtifactError> {
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.client
                    .create_bucket()
                    .bucket(&self.bucket)
                    .send()
                    .await
                    .map_err(|e| ArtifactError::Storage(format!("create bucket: {e}")))?;
                Ok(())
            }
        }
    }
}

#[async_trait]
impl ArtifactStore for S3Store {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    #[instrument(skip(self, bytes), fields(bucket = %self.bucket))]
    async fn put(&self, bytes: Bytes) -> Result<Sha256, ArtifactError> {
        let sha = Sha256::compute(&bytes);
        let key = object_key(&sha);

        if self.exists(&sha).await? {
            return Ok(sha);
        }

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(|e| ArtifactError::Storage(format!("put_object: {e}")))?;

        Ok(sha)
    }

    async fn get(&self, sha: &Sha256) -> Result<Bytes, ArtifactError> {
        let key = object_key(sha);
        let out = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| {
                if e.as_service_error().is_some_and(|s| s.is_no_such_key()) {
                    ArtifactError::NotFound
                } else {
                    ArtifactError::Storage(format!("get_object: {e}"))
                }
            })?;

        let aggregated = out
            .body
            .collect()
            .await
            .map_err(|e| ArtifactError::Storage(format!("read body: {e}")))?;
        Ok(aggregated.into_bytes())
    }

    async fn exists(&self, sha: &Sha256) -> Result<bool, ArtifactError> {
        let key = object_key(sha);
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) if e.as_service_error().is_some_and(|s| s.is_not_found()) => Ok(false),
            Err(e) => Err(ArtifactError::Storage(format!("head_object: {e}"))),
        }
    }

    async fn delete(&self, sha: &Sha256) -> Result<(), ArtifactError> {
        let key = object_key(sha);
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| ArtifactError::Storage(format!("delete_object: {e}")))?;
        Ok(())
    }
}

/// Build an [`S3Store`] from standard `S3_*` environment variables.
pub async fn from_env() -> Result<Arc<S3Store>, ArtifactError> {
    fn require(name: &'static str) -> Result<String, ArtifactError> {
        std::env::var(name).map_err(|_| ArtifactError::Storage(format!("missing env {name}")))
    }
    let config = S3Config {
        endpoint: require("S3_ENDPOINT")?,
        bucket: require("S3_BUCKET")?,
        region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        access_key: require("S3_ACCESS_KEY_ID")?,
        secret_key: require("S3_SECRET_ACCESS_KEY")?,
    };
    Ok(Arc::new(S3Store::new(config).await?))
}
