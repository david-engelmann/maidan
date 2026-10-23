//! Workspace key custodies beyond the instance KEK: AWS KMS, and a fake KMS
//! for tests. [`maidan_types::WorkspaceKeyCustody`] is the interface; the
//! keyring opens every workspace key through the custody whose wrapping key
//! id the row records, before a read maps its rows.
//!
//! Each workspace key gets a KMS key of its own. Destroying that KMS key
//! (on erasure, or once rotation has moved every data key off it) makes the
//! stored form, and any backup of it, unopenable: no other key in the KMS
//! can decrypt it.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::RwLock;

use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::Tag;
use maidan_types::{
    ContentKey, CustodyFuture, LocalCustody, SealError, WorkspaceKeyCustody, WrappedKey,
};
use uuid::Uuid;

/// Prefix of the wrapping key id an AWS KMS wrap records:
/// `aws-kms:<key ARN>`.
pub const AWS_KMS_PREFIX: &str = "aws-kms:";
/// Prefix of the wrapping key id a [`FakeKmsCustody`] wrap records:
/// `fake-kms:<name>:<workspace key id>`.
pub const FAKE_KMS_PREFIX: &str = "fake-kms:";
/// The encryption context key (and KMS key tag) that binds a wrap to its
/// workspace key, so a wrapped key copied onto another row does not decrypt.
const CONTEXT_KEY: &str = "maidan_workspace_key";
/// The shortest waiting period AWS KMS allows before it deletes a key. The
/// key is disabled at once, so nothing decrypts with it in the meantime.
const DELETION_WINDOW_DAYS: i32 = 7;

/// Workspace keys each wrapped by a KMS key of their own, created on first
/// wrap with `CreateKey` and tagged with the workspace key id, then used
/// through `Encrypt`/`Decrypt` with that id as encryption context.
/// Destroying one disables its KMS key and schedules its deletion.
pub struct AwsKmsCustody {
    client: aws_sdk_kms::Client,
}

impl AwsKmsCustody {
    pub fn new(client: aws_sdk_kms::Client) -> Self {
        Self { client }
    }

    /// A client from the default AWS configuration chain (region,
    /// credentials, endpoint override).
    pub async fn from_default_config() -> Self {
        use aws_smithy_http_client::tls;
        let https = aws_smithy_http_client::Builder::new()
            .tls_provider(tls::Provider::Rustls(
                tls::rustls_provider::CryptoMode::Ring,
            ))
            .build_https();
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .http_client(https)
            .load()
            .await;
        Self::new(aws_sdk_kms::Client::new(&shared))
    }

    fn kms_error(op: &str, err: impl fmt::Display) -> SealError {
        // The SDK error names the operation and the AWS code; it never carries
        // key material.
        tracing::warn!(operation = op, error = %err, "AWS KMS call failed");
        SealError::Authentication
    }

    fn key_arn(wrapped: &WrappedKey) -> Result<&str, SealError> {
        wrapped
            .kek_id
            .strip_prefix(AWS_KMS_PREFIX)
            .ok_or_else(|| SealError::UnknownKek(wrapped.kek_id.clone()))
    }
}

impl WorkspaceKeyCustody for AwsKmsCustody {
    fn name(&self) -> &'static str {
        "aws-kms"
    }

    fn current_wrapping_like(&self) -> String {
        format!("{AWS_KMS_PREFIX}%")
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
            let tag = Tag::builder()
                .tag_key(CONTEXT_KEY)
                .tag_value(workspace_key_id.to_string())
                .build()
                .map_err(|err| Self::kms_error("CreateKey", err))?;
            let created = self
                .client
                .create_key()
                .description(format!("Maidan workspace key {workspace_key_id}"))
                .tags(tag)
                .send()
                .await
                .map_err(|err| Self::kms_error("CreateKey", err))?;
            let arn = created
                .key_metadata
                .map(|meta| meta.arn.unwrap_or(meta.key_id))
                .ok_or(SealError::Malformed)?;
            let out = self
                .client
                .encrypt()
                .key_id(&arn)
                .plaintext(Blob::new(key.as_bytes().to_vec()))
                .encryption_context(CONTEXT_KEY, workspace_key_id.to_string())
                .send()
                .await
                .map_err(|err| Self::kms_error("Encrypt", err))?;
            let blob = out.ciphertext_blob.ok_or(SealError::Malformed)?;
            Ok(WrappedKey {
                kek_id: format!("{AWS_KMS_PREFIX}{arn}"),
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
            let arn = Self::key_arn(wrapped)?;
            let out = self
                .client
                .decrypt()
                .key_id(arn)
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

    fn destroy<'a>(
        &'a self,
        _workspace_key_id: Uuid,
        wrapped: &'a WrappedKey,
    ) -> CustodyFuture<'a, ()> {
        Box::pin(async move {
            let arn = Self::key_arn(wrapped)?;
            self.client
                .disable_key()
                .key_id(arn)
                .send()
                .await
                .map_err(|err| Self::kms_error("DisableKey", err))?;
            self.client
                .schedule_key_deletion()
                .key_id(arn)
                .pending_window_in_days(DELETION_WINDOW_DAYS)
                .send()
                .await
                .map_err(|err| Self::kms_error("ScheduleKeyDeletion", err))?;
            Ok(())
        })
    }
}

impl fmt::Debug for AwsKmsCustody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsKmsCustody").finish_non_exhaustive()
    }
}

/// A KMS stand-in for tests: one random key of its own per workspace key,
/// under `fake-kms:<name>:<workspace key id>`. It counts its calls, can be
/// told to refuse chosen workspace keys (as a KMS does when a key policy or
/// grant is revoked), and destroying a key forgets it for good.
pub struct FakeKmsCustody {
    name: String,
    keys: RwLock<HashMap<Uuid, LocalCustody>>,
    refused: RwLock<HashSet<Uuid>>,
    unwraps: std::sync::atomic::AtomicUsize,
}

impl FakeKmsCustody {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            keys: RwLock::default(),
            refused: RwLock::default(),
            unwraps: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Refuse every later call about this workspace key.
    pub fn refuse(&self, workspace_key_id: Uuid) {
        if let Ok(mut refused) = self.refused.write() {
            refused.insert(workspace_key_id);
        }
    }

    /// Serve this workspace key again after [`Self::refuse`].
    pub fn allow(&self, workspace_key_id: Uuid) {
        if let Ok(mut refused) = self.refused.write() {
            refused.remove(&workspace_key_id);
        }
    }

    /// How many unwraps reached this custody.
    pub fn unwraps(&self) -> usize {
        self.unwraps.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether the custody still holds a key for this workspace key.
    pub fn holds(&self, workspace_key_id: Uuid) -> bool {
        self.keys
            .read()
            .is_ok_and(|keys| keys.contains_key(&workspace_key_id))
    }

    fn wrapping_id(&self, workspace_key_id: Uuid) -> String {
        format!("{FAKE_KMS_PREFIX}{}:{workspace_key_id}", self.name)
    }

    fn check(&self, workspace_key_id: Uuid) -> Result<(), SealError> {
        match self.refused.read() {
            Ok(refused) if !refused.contains(&workspace_key_id) => Ok(()),
            _ => Err(SealError::Authentication),
        }
    }

    fn wrap_now(&self, workspace_key_id: Uuid, key: &ContentKey) -> Result<WrappedKey, SealError> {
        self.check(workspace_key_id)?;
        let own = LocalCustody::new(*ContentKey::generate().as_bytes(), Vec::new());
        let sealed = own.seal(workspace_key_id, key)?;
        self.keys
            .write()
            .map_err(|_| SealError::Authentication)?
            .insert(workspace_key_id, own);
        Ok(WrappedKey {
            kek_id: self.wrapping_id(workspace_key_id),
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
        if wrapped.kek_id != self.wrapping_id(workspace_key_id) {
            return Err(SealError::UnknownKek(wrapped.kek_id.clone()));
        }
        let keys = self.keys.read().map_err(|_| SealError::Authentication)?;
        let own = keys
            .get(&workspace_key_id)
            .ok_or_else(|| SealError::UnknownKek(wrapped.kek_id.clone()))?;
        own.open(
            workspace_key_id,
            &WrappedKey {
                kek_id: own.wrapping_id().to_string(),
                blob: wrapped.blob.clone(),
            },
        )
    }

    fn destroy_now(&self, workspace_key_id: Uuid) -> Result<(), SealError> {
        self.check(workspace_key_id)?;
        self.keys
            .write()
            .map_err(|_| SealError::Authentication)?
            .remove(&workspace_key_id);
        Ok(())
    }
}

impl WorkspaceKeyCustody for FakeKmsCustody {
    fn name(&self) -> &'static str {
        "fake-kms"
    }

    fn current_wrapping_like(&self) -> String {
        format!("{FAKE_KMS_PREFIX}{}:%", self.name)
    }

    fn owns(&self, wrapping_key_id: &str) -> bool {
        wrapping_key_id
            .strip_prefix(FAKE_KMS_PREFIX)
            .and_then(|rest| rest.strip_prefix(self.name.as_str()))
            .is_some_and(|rest| rest.starts_with(':'))
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

    fn destroy<'a>(
        &'a self,
        workspace_key_id: Uuid,
        _wrapped: &'a WrappedKey,
    ) -> CustodyFuture<'a, ()> {
        let result = self.destroy_now(workspace_key_id);
        Box::pin(async move {
            tokio::task::yield_now().await;
            result
        })
    }
}

impl fmt::Debug for FakeKmsCustody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeKmsCustody")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_fake_wrap_opens_only_for_its_own_workspace_key_until_destroyed() {
        let kms = FakeKmsCustody::new("t");
        let (mine, theirs) = (Uuid::now_v7(), Uuid::now_v7());
        let key = ContentKey::generate();
        let wrapped = kms.wrap(mine, &key).await.unwrap();
        assert_eq!(wrapped.kek_id, format!("fake-kms:t:{mine}"));
        assert_eq!(kms.unwrap(mine, &wrapped).await.unwrap(), key);
        assert!(kms.unwrap(theirs, &wrapped).await.is_err());
        let copy = wrapped.clone();
        kms.destroy(mine, &wrapped).await.unwrap();
        assert!(!kms.holds(mine));
        assert!(kms.unwrap(mine, &copy).await.is_err());
    }

    #[tokio::test]
    async fn a_refused_fake_key_neither_opens_nor_destroys() {
        let kms = FakeKmsCustody::new("t");
        let mine = Uuid::now_v7();
        let wrapped = kms.wrap(mine, &ContentKey::generate()).await.unwrap();
        kms.refuse(mine);
        assert_eq!(
            kms.unwrap(mine, &wrapped).await,
            Err(SealError::Authentication)
        );
        assert_eq!(
            kms.destroy(mine, &wrapped).await,
            Err(SealError::Authentication)
        );
        assert!(kms.holds(mine));
    }
}
