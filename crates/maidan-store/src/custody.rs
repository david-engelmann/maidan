//! Workspace key custodies beyond the instance KEK: AWS KMS, and a fake KMS
//! for tests. [`maidan_types::WorkspaceKeyCustody`] is the interface; the
//! keyring opens every workspace key through the custody whose wrapping key
//! id the row records, before a read maps its rows.

use std::collections::HashSet;
use std::fmt;
use std::sync::RwLock;

use aws_sdk_kms::primitives::Blob;
use maidan_types::{
    ContentKey, CustodyFuture, LocalCustody, SealError, WorkspaceKeyCustody, WrappedKey,
};
use uuid::Uuid;

/// Prefix of the wrapping key id an AWS KMS wrap records.
pub const AWS_KMS_PREFIX: &str = "aws-kms:";
/// Prefix of the wrapping key id a [`FakeKmsCustody`] wrap records.
pub const FAKE_KMS_PREFIX: &str = "fake-kms:";
/// The encryption context key that binds a wrap to its workspace key, so a
/// wrapped key copied onto another row does not decrypt.
const CONTEXT_KEY: &str = "maidan_workspace_key";

/// Workspace keys wrapped by one AWS KMS key with `Encrypt`/`Decrypt`. The
/// wrapping key id is `aws-kms:<key id or ARN>`; changing the configured key
/// makes the rewrap pass move every row to it (the old key must still allow
/// `Decrypt` until it has).
pub struct AwsKmsCustody {
    client: aws_sdk_kms::Client,
    key_id: String,
    wrapping_id: String,
}

impl AwsKmsCustody {
    pub fn new(client: aws_sdk_kms::Client, key_id: impl Into<String>) -> Self {
        let key_id = key_id.into();
        Self {
            client,
            wrapping_id: format!("{AWS_KMS_PREFIX}{key_id}"),
            key_id,
        }
    }

    /// A client from the default AWS configuration chain (region,
    /// credentials, endpoint override).
    pub async fn from_default_config(key_id: impl Into<String>) -> Self {
        use aws_smithy_http_client::tls;
        let https = aws_smithy_http_client::Builder::new()
            .tls_provider(tls::Provider::Rustls(tls::rustls_provider::CryptoMode::Ring))
            .build_https();
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .http_client(https)
            .load()
            .await;
        Self::new(aws_sdk_kms::Client::new(&shared), key_id)
    }

    fn kms_error(op: &str, err: impl fmt::Display) -> SealError {
        // The SDK error names the operation and the AWS code; it never carries
        // key material.
        tracing::warn!(operation = op, error = %err, "AWS KMS call failed");
        SealError::Authentication
    }
}

impl WorkspaceKeyCustody for AwsKmsCustody {
    fn name(&self) -> &'static str {
        "aws-kms"
    }

    fn wrapping_id(&self) -> &str {
        &self.wrapping_id
    }

    fn owns(&self, wrapping_key_id: &str) -> bool {
        wrapping_key_id.starts_with(AWS_KMS_PREFIX)
    }

    fn wrap<'a>(
        &'a self,
        workspace_key_id: Uuid,
        key: &'a ContentKey,
    ) -> CustodyFuture<'a, WrappedKey> {
        Box::pin(async move {
            let out = self
                .client
                .encrypt()
                .key_id(&self.key_id)
                .plaintext(Blob::new(key.as_bytes().to_vec()))
                .encryption_context(CONTEXT_KEY, workspace_key_id.to_string())
                .send()
                .await
                .map_err(|err| Self::kms_error("Encrypt", err))?;
            let blob = out.ciphertext_blob.ok_or(SealError::Malformed)?;
            Ok(WrappedKey {
                kek_id: self.wrapping_id.clone(),
                blob: blob.into_inner(),
            })
        })
    }

    fn unwrap<'a>(
        &'a self,
        workspace_key_id: Uuid,
        wrapped: &'a WrappedKey,
    ) -> CustodyFuture<'a, ContentKey> {
        Box::pin(async move {
            let key_id = wrapped
                .kek_id
                .strip_prefix(AWS_KMS_PREFIX)
                .ok_or_else(|| SealError::UnknownKek(wrapped.kek_id.clone()))?;
            let out = self
                .client
                .decrypt()
                .key_id(key_id)
                .ciphertext_blob(Blob::new(wrapped.blob.clone()))
                .encryption_context(CONTEXT_KEY, workspace_key_id.to_string())
                .send()
                .await
                .map_err(|err| Self::kms_error("Decrypt", err))?;
            let plain = out.plaintext.ok_or(SealError::Malformed)?.into_inner();
            let bytes: [u8; 32] = plain
                .as_slice()
                .try_into()
                .map_err(|_| SealError::Malformed)?;
            Ok(ContentKey::from_bytes(bytes))
        })
    }
}

impl fmt::Debug for AwsKmsCustody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsKmsCustody")
            .field("key_id", &self.key_id)
            .finish()
    }
}

/// A KMS stand-in for tests: wraps with a random key of its own under
/// `fake-kms:<name>`, counts its calls, and can be told to refuse chosen
/// workspace keys, as a KMS does when a key policy or grant is revoked.
pub struct FakeKmsCustody {
    inner: LocalCustody,
    wrapping_id: String,
    refused: RwLock<HashSet<Uuid>>,
    unwraps: std::sync::atomic::AtomicUsize,
}

impl FakeKmsCustody {
    pub fn new(name: &str) -> Self {
        Self {
            inner: LocalCustody::new(*ContentKey::generate().as_bytes(), Vec::new()),
            wrapping_id: format!("{FAKE_KMS_PREFIX}{name}"),
            refused: RwLock::default(),
            unwraps: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Refuse every later wrap and unwrap of this workspace key.
    pub fn refuse(&self, workspace_key_id: Uuid) {
        if let Ok(mut refused) = self.refused.write() {
            refused.insert(workspace_key_id);
        }
    }

    /// How many unwraps reached this custody.
    pub fn unwraps(&self) -> usize {
        self.unwraps.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn check(&self, workspace_key_id: Uuid) -> Result<(), SealError> {
        match self.refused.read() {
            Ok(refused) if !refused.contains(&workspace_key_id) => Ok(()),
            _ => Err(SealError::Authentication),
        }
    }

    fn wrap_now(&self, workspace_key_id: Uuid, key: &ContentKey) -> Result<WrappedKey, SealError> {
        self.check(workspace_key_id)?;
        let sealed = self.inner.seal(workspace_key_id, key)?;
        Ok(WrappedKey {
            kek_id: self.wrapping_id.clone(),
            blob: sealed.blob,
        })
    }

    fn unwrap_now(
        &self,
        workspace_key_id: Uuid,
        wrapped: &WrappedKey,
    ) -> Result<ContentKey, SealError> {
        self.unwraps
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.check(workspace_key_id)?;
        if wrapped.kek_id != self.wrapping_id {
            return Err(SealError::UnknownKek(wrapped.kek_id.clone()));
        }
        self.inner.open(
            workspace_key_id,
            &WrappedKey {
                kek_id: self.inner.wrapping_id().to_string(),
                blob: wrapped.blob.clone(),
            },
        )
    }
}

impl WorkspaceKeyCustody for FakeKmsCustody {
    fn name(&self) -> &'static str {
        "fake-kms"
    }

    fn wrapping_id(&self) -> &str {
        &self.wrapping_id
    }

    fn owns(&self, wrapping_key_id: &str) -> bool {
        wrapping_key_id == self.wrapping_id
    }

    fn wrap<'a>(
        &'a self,
        workspace_key_id: Uuid,
        key: &'a ContentKey,
    ) -> CustodyFuture<'a, WrappedKey> {
        let result = self.wrap_now(workspace_key_id, key);
        Box::pin(async move {
            // A real KMS is a network hop: yield, so a caller that holds a
            // lock across the call shows up in tests.
            tokio::task::yield_now().await;
            result
        })
    }

    fn unwrap<'a>(
        &'a self,
        workspace_key_id: Uuid,
        wrapped: &'a WrappedKey,
    ) -> CustodyFuture<'a, ContentKey> {
        let result = self.unwrap_now(workspace_key_id, wrapped);
        Box::pin(async move {
            tokio::task::yield_now().await;
            result
        })
    }
}

impl fmt::Debug for FakeKmsCustody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeKmsCustody")
            .field("wrapping_id", &self.wrapping_id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_fake_wrap_opens_only_for_its_own_workspace_key() {
        let kms = FakeKmsCustody::new("t");
        let (mine, theirs) = (Uuid::now_v7(), Uuid::now_v7());
        let key = ContentKey::generate();
        let wrapped = kms.wrap(mine, &key).await.unwrap();
        assert_eq!(wrapped.kek_id, "fake-kms:t");
        assert_eq!(kms.unwrap(mine, &wrapped).await.unwrap(), key);
        assert!(kms.unwrap(theirs, &wrapped).await.is_err());
        kms.refuse(mine);
        assert_eq!(
            kms.unwrap(mine, &wrapped).await,
            Err(SealError::Authentication)
        );
    }
}
