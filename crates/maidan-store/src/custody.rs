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

use aws_sdk_kms::error::ProvideErrorMetadata;
use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::Tag;
use chrono::{DateTime, Utc};
use maidan_types::{
    ContentKey, CustodyFuture, CustodyKey, LocalCustody, SealError, WorkspaceKeyCustody, WrappedKey,
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
/// The KMS key tag naming the Maidan instance that created a key, so the
/// stray-key sweep only ever lists its own instance's keys in a shared
/// account.
const INSTANCE_TAG: &str = "maidan_instance";

/// Workspace keys each wrapped by a KMS key of their own, created on first
/// wrap with `CreateKey` and tagged with the workspace key id, then used
/// through `Encrypt`/`Decrypt` with that id as encryption context.
/// Destroying one disables its KMS key and schedules its deletion.
///
/// With an instance name, each key is also tagged with it, and
/// [`WorkspaceKeyCustody::held_keys`] lists the keys carrying both tags.
/// Without one, keys are not tagged with an instance and nothing is listed,
/// so the stray-key sweep never touches a key it cannot prove is its own.
pub struct AwsKmsCustody {
    client: aws_sdk_kms::Client,
    instance: Option<String>,
}

impl AwsKmsCustody {
    pub fn new(client: aws_sdk_kms::Client, instance: Option<String>) -> Self {
        Self { client, instance }
    }

    /// A client from the default AWS configuration chain (region,
    /// credentials, endpoint override).
    pub async fn from_default_config(instance: Option<String>) -> Self {
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
        Self::new(aws_sdk_kms::Client::new(&shared), instance)
    }

    /// The listed key, when it is one of this instance's enabled workspace
    /// keys created before `created_before`. A key this role may not read
    /// the tags of belongs to someone else: skipped, not an error.
    async fn own_key(
        &self,
        instance: &str,
        key_id: &str,
        created_before: DateTime<Utc>,
    ) -> Result<Option<CustodyKey>, SealError> {
        let tags = match self.client.list_resource_tags().key_id(key_id).send().await {
            Ok(out) => out.tags.unwrap_or_default(),
            Err(err) if kms_error_kind(err.code()) == SealError::Refused => return Ok(None),
            Err(err) => return Err(Self::kms_error("ListResourceTags", err)),
        };
        let tag = |name: &str| {
            tags.iter()
                .find(|t| t.tag_key() == name)
                .map(|t| t.tag_value().to_string())
        };
        if tag(INSTANCE_TAG).as_deref() != Some(instance) {
            return Ok(None);
        }
        let Some(workspace_key_id) = tag(CONTEXT_KEY).and_then(|v| v.parse::<Uuid>().ok()) else {
            return Ok(None);
        };
        let described = self.client.describe_key().key_id(key_id).send().await;
        // A key this role may not describe is someone else's: skipped, like
        // one whose tags it may not read, so one key never stops the sweep.
        let described = match described {
            Ok(out) => out,
            Err(err) if kms_error_kind(err.code()) == SealError::Refused => return Ok(None),
            Err(err) => return Err(Self::kms_error("DescribeKey", err)),
        };
        let Some(meta) = described.key_metadata else {
            return Ok(None);
        };
        let enabled = meta.key_state == Some(aws_sdk_kms::types::KeyState::Enabled);
        let old_enough = meta
            .creation_date
            .is_some_and(|created| created.secs() < created_before.timestamp());
        if !(enabled && old_enough) {
            return Ok(None);
        }
        let arn = meta.arn.unwrap_or(meta.key_id);
        Ok(Some(CustodyKey {
            workspace_key_id,
            wrapping_key_id: format!("{AWS_KMS_PREFIX}{arn}"),
        }))
    }

    fn kms_error(op: &str, err: impl fmt::Display + ProvideErrorMetadata) -> SealError {
        // The SDK error names the operation and the AWS code; it never carries
        // key material.
        tracing::warn!(operation = op, error = %err, "AWS KMS call failed");
        kms_error_kind(err.code())
    }

    fn key_arn(wrapped: &WrappedKey) -> Result<&str, SealError> {
        wrapped
            .kek_id
            .strip_prefix(AWS_KMS_PREFIX)
            .ok_or_else(|| SealError::UnknownKek(wrapped.kek_id.clone()))
    }
}

/// KMS error codes that mean the custody refuses this key until an operator
/// acts. Anything else, including a call that never got an answer, is
/// transient.
const KMS_REFUSALS: &[&str] = &[
    "AccessDeniedException",
    "DisabledException",
    "IncorrectKeyException",
    "InvalidCiphertextException",
    "InvalidKeyUsageException",
    "KMSInvalidStateException",
    "NotFoundException",
];

/// Sort a KMS error by its AWS code: a refusal, or worth retrying.
fn kms_error_kind(code: Option<&str>) -> SealError {
    match code {
        Some(code) if KMS_REFUSALS.contains(&code) => SealError::Refused,
        _ => SealError::Unavailable,
    }
}

impl WorkspaceKeyCustody for AwsKmsCustody {
    fn name(&self) -> &'static str {
        "aws-kms"
    }

    fn current_wrapping_like(&self) -> String {
        format!("{AWS_KMS_PREFIX}%")
    }

    fn creates_keys(&self) -> bool {
        true
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
                .map_err(|_| SealError::Malformed)?;
            let mut create = self
                .client
                .create_key()
                .description(format!("Maidan workspace key {workspace_key_id}"))
                .tags(tag);
            if let Some(instance) = &self.instance {
                create = create.tags(
                    Tag::builder()
                        .tag_key(INSTANCE_TAG)
                        .tag_value(instance)
                        .build()
                        .map_err(|_| SealError::Malformed)?,
                );
            }
            let created = create
                .send()
                .await
                .map_err(|err| Self::kms_error("CreateKey", err))?;
            let arn = created
                .key_metadata
                .map(|meta| meta.arn.unwrap_or(meta.key_id))
                .ok_or(SealError::Malformed)?;
            let encrypted = self
                .client
                .encrypt()
                .key_id(&arn)
                .plaintext(Blob::new(key.as_bytes().to_vec()))
                .encryption_context(CONTEXT_KEY, workspace_key_id.to_string())
                .send()
                .await;
            let out = match encrypted {
                Ok(out) => out,
                Err(err) => {
                    // Nothing will ever point at the key just created: schedule
                    // its deletion rather than leave it billing.
                    if let Err(cleanup) = self
                        .client
                        .schedule_key_deletion()
                        .key_id(&arn)
                        .pending_window_in_days(DELETION_WINDOW_DAYS)
                        .send()
                        .await
                    {
                        tracing::warn!(error = %cleanup, "could not schedule deletion of an unused KMS key");
                    }
                    return Err(Self::kms_error("Encrypt", err));
                }
            };
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
            // A key pending deletion decrypts nothing, so scheduling the
            // deletion is the whole destroy. Destroying is idempotent: a key
            // already pending deletion, or already gone, counts as destroyed.
            let scheduled = self
                .client
                .schedule_key_deletion()
                .key_id(arn)
                .pending_window_in_days(DELETION_WINDOW_DAYS)
                .send()
                .await;
            let err = match scheduled {
                Ok(_) => return Ok(()),
                Err(err) => err,
            };
            match err.as_service_error() {
                Some(e) if e.is_not_found_exception() => return Ok(()),
                Some(e) if e.is_kms_invalid_state_exception() => {
                    let described = self
                        .client
                        .describe_key()
                        .key_id(arn)
                        .send()
                        .await
                        .map_err(|err| Self::kms_error("DescribeKey", err))?;
                    let pending = described
                        .key_metadata()
                        .and_then(|m| m.key_state())
                        .is_some_and(|s| *s == aws_sdk_kms::types::KeyState::PendingDeletion);
                    if pending {
                        return Ok(());
                    }
                }
                _ => {}
            }
            Err(Self::kms_error("ScheduleKeyDeletion", err))?;
            Ok(())
        })
    }

    fn held_keys<'a>(
        &'a self,
        created_before: DateTime<Utc>,
    ) -> CustodyFuture<'a, Vec<CustodyKey>> {
        Box::pin(async move {
            let Some(instance) = self.instance.as_deref() else {
                return Ok(Vec::new());
            };
            let mut held = Vec::new();
            let mut marker: Option<String> = None;
            loop {
                let page = self
                    .client
                    .list_keys()
                    .set_marker(marker.take())
                    .send()
                    .await
                    .map_err(|err| Self::kms_error("ListKeys", err))?;
                for entry in page.keys() {
                    let Some(key_id) = entry.key_id() else {
                        continue;
                    };
                    if let Some(key) = self.own_key(instance, key_id, created_before).await? {
                        held.push(key);
                    }
                }
                match (page.truncated, page.next_marker) {
                    (true, Some(next)) => marker = Some(next),
                    _ => break,
                }
            }
            Ok(held)
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
    created: RwLock<HashMap<Uuid, DateTime<Utc>>>,
    refused: RwLock<HashSet<Uuid>>,
    unavailable: RwLock<HashMap<Uuid, usize>>,
    unwraps: std::sync::atomic::AtomicUsize,
}

impl FakeKmsCustody {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            keys: RwLock::default(),
            created: RwLock::default(),
            refused: RwLock::default(),
            unavailable: RwLock::default(),
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

    /// Fail the next `times` calls about this workspace key as a KMS does
    /// when throttled or unreachable.
    pub fn fail_transiently(&self, workspace_key_id: Uuid, times: usize) {
        if let Ok(mut down) = self.unavailable.write() {
            down.insert(workspace_key_id, times);
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
        let mut down = self
            .unavailable
            .write()
            .map_err(|_| SealError::Unavailable)?;
        if let Some(left) = down.get_mut(&workspace_key_id).filter(|left| **left > 0) {
            *left -= 1;
            return Err(SealError::Unavailable);
        }
        drop(down);
        match self.refused.read() {
            Ok(refused) if !refused.contains(&workspace_key_id) => Ok(()),
            _ => Err(SealError::Refused),
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
        self.created
            .write()
            .map_err(|_| SealError::Authentication)?
            .insert(workspace_key_id, Utc::now());
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
        self.created
            .write()
            .map_err(|_| SealError::Authentication)?
            .remove(&workspace_key_id);
        Ok(())
    }

    fn held_now(&self, created_before: DateTime<Utc>) -> Result<Vec<CustodyKey>, SealError> {
        let created = self.created.read().map_err(|_| SealError::Unavailable)?;
        Ok(created
            .iter()
            .filter(|(_, at)| **at < created_before)
            .map(|(id, _)| CustodyKey {
                workspace_key_id: *id,
                wrapping_key_id: self.wrapping_id(*id),
            })
            .collect())
    }
}

impl WorkspaceKeyCustody for FakeKmsCustody {
    fn name(&self) -> &'static str {
        "fake-kms"
    }

    fn current_wrapping_like(&self) -> String {
        format!("{FAKE_KMS_PREFIX}{}:%", self.name)
    }

    fn creates_keys(&self) -> bool {
        true
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

    fn held_keys<'a>(
        &'a self,
        created_before: DateTime<Utc>,
    ) -> CustodyFuture<'a, Vec<CustodyKey>> {
        let result = self.held_now(created_before);
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
        assert_eq!(kms.unwrap(mine, &wrapped).await, Err(SealError::Refused));
        assert_eq!(kms.destroy(mine, &wrapped).await, Err(SealError::Refused));
        assert!(kms.holds(mine));
    }

    #[tokio::test]
    async fn a_transient_fake_failure_is_unavailable_not_refused() {
        let kms = FakeKmsCustody::new("t");
        let mine = Uuid::now_v7();
        let wrapped = kms.wrap(mine, &ContentKey::generate()).await.unwrap();
        kms.fail_transiently(mine, 1);
        assert_eq!(
            kms.unwrap(mine, &wrapped).await,
            Err(SealError::Unavailable)
        );
        assert!(kms.unwrap(mine, &wrapped).await.is_ok());
    }

    #[tokio::test]
    async fn a_fake_custody_lists_the_keys_it_holds_created_before_the_cutoff() {
        let kms = FakeKmsCustody::new("t");
        let (kept, gone) = (Uuid::now_v7(), Uuid::now_v7());
        kms.wrap(kept, &ContentKey::generate()).await.unwrap();
        let wrapped = kms.wrap(gone, &ContentKey::generate()).await.unwrap();
        kms.destroy(gone, &wrapped).await.unwrap();
        let later = Utc::now() + chrono::Duration::seconds(1);
        assert_eq!(
            kms.held_keys(later).await.unwrap(),
            vec![CustodyKey {
                workspace_key_id: kept,
                wrapping_key_id: format!("fake-kms:t:{kept}"),
            }]
        );
        let earlier = Utc::now() - chrono::Duration::hours(1);
        assert!(kms.held_keys(earlier).await.unwrap().is_empty());
    }

    type Reply = dyn Fn(
            http::Request<aws_smithy_types::body::SdkBody>,
        ) -> http::Response<aws_smithy_types::body::SdkBody>
        + Send
        + Sync;

    /// An HTTP client that answers every request from a closure.
    #[derive(Clone)]
    struct FnClient(std::sync::Arc<Reply>);

    impl std::fmt::Debug for FnClient {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FnClient")
        }
    }

    impl FnClient {
        fn shared(
            f: impl Fn(
                    http::Request<aws_smithy_types::body::SdkBody>,
                ) -> http::Response<aws_smithy_types::body::SdkBody>
                + Send
                + Sync
                + 'static,
        ) -> aws_smithy_runtime_api::client::http::SharedHttpClient {
            aws_smithy_runtime_api::client::http::SharedHttpClient::new(FnClient(
                std::sync::Arc::new(f),
            ))
        }
    }

    impl aws_smithy_runtime_api::client::http::HttpConnector for FnClient {
        fn call(
            &self,
            request: aws_smithy_runtime_api::client::orchestrator::HttpRequest,
        ) -> aws_smithy_runtime_api::client::http::HttpConnectorFuture {
            let response = (self.0)(request.try_into_http1x().unwrap());
            aws_smithy_runtime_api::client::http::HttpConnectorFuture::ready(Ok(
                aws_smithy_runtime_api::client::orchestrator::HttpResponse::try_from(response)
                    .unwrap(),
            ))
        }
    }

    impl aws_smithy_runtime_api::client::http::HttpClient for FnClient {
        fn http_connector(
            &self,
            _: &aws_smithy_runtime_api::client::http::HttpConnectorSettings,
            _: &aws_smithy_runtime_api::client::runtime_components::RuntimeComponents,
        ) -> aws_smithy_runtime_api::client::http::SharedHttpConnector {
            aws_smithy_runtime_api::client::http::SharedHttpConnector::new(self.clone())
        }
    }

    /// A KMS account shared by three deployments: `prod`, `staging`, and one
    /// that set no instance name. Answers ListKeys, ListResourceTags and
    /// DescribeKey from the table below.
    fn shared_account_kms(instance: Option<&str>) -> AwsKmsCustody {
        use aws_smithy_types::body::SdkBody;
        let client = FnClient::shared(|req: http::Request<SdkBody>| {
            let target = req
                .headers()
                .get("x-amz-target")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let body = String::from_utf8_lossy(req.body().bytes().unwrap_or_default()).to_string();
            let wk = |n: u8| format!("0190a000-0000-7000-8000-00000000000{n}");
            let tags = |key: &str| match key {
                "k-prod" => format!(
                    r#"[{{"TagKey":"maidan_instance","TagValue":"prod"}},{{"TagKey":"maidan_workspace_key","TagValue":"{}"}}]"#,
                    wk(1)
                ),
                "k-staging" => format!(
                    r#"[{{"TagKey":"maidan_instance","TagValue":"staging"}},{{"TagKey":"maidan_workspace_key","TagValue":"{}"}}]"#,
                    wk(2)
                ),
                "k-denied" => format!(
                    r#"[{{"TagKey":"maidan_instance","TagValue":"prod"}},{{"TagKey":"maidan_workspace_key","TagValue":"{}"}}]"#,
                    wk(4)
                ),
                "k-untagged" => format!(
                    r#"[{{"TagKey":"maidan_workspace_key","TagValue":"{}"}}]"#,
                    wk(3)
                ),
                _ => "[]".to_string(),
            };
            let key = ["k-prod", "k-staging", "k-denied", "k-untagged", "k-other"]
                .into_iter()
                .find(|k| body.contains(&format!("\"{k}\"")))
                .unwrap_or("k-other");
            let json = match target.as_str() {
                    "TrentService.ListKeys" => r#"{"Keys":[{"KeyId":"k-prod"},{"KeyId":"k-staging"},{"KeyId":"k-denied"},{"KeyId":"k-untagged"},{"KeyId":"k-other"}],"Truncated":false}"#.to_string(),
                    "TrentService.ListResourceTags" => format!(r#"{{"Tags":{},"Truncated":false}}"#, tags(key)),
                    "TrentService.DescribeKey" => format!(
                        r#"{{"KeyMetadata":{{"KeyId":"{key}","Arn":"arn:aws:kms:us-east-1:1:key/{key}","KeyState":"Enabled","CreationDate":1600000000}}}}"#
                    ),
                    _ => "{}".to_string(),
                };
            // The role may read k-denied's tags but not describe it.
            let (status, json) = if target == "TrentService.DescribeKey" && key == "k-denied" {
                (
                    400,
                    r#"{"__type":"AccessDeniedException","message":"denied"}"#.to_string(),
                )
            } else {
                (200, json)
            };
            http::Response::builder()
                .status(status)
                .header("content-type", "application/x-amz-json-1.1")
                .body(SdkBody::from(json))
                .unwrap()
        });
        let config = aws_sdk_kms::Config::builder()
            .behavior_version(aws_sdk_kms::config::BehaviorVersion::latest())
            .region(aws_sdk_kms::config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_kms::config::Credentials::new(
                "a", "b", None, None, "test",
            ))
            .http_client(client)
            .build();
        AwsKmsCustody::new(
            aws_sdk_kms::Client::from_conf(config),
            instance.map(str::to_string),
        )
    }

    #[tokio::test]
    async fn the_sweep_lists_only_keys_tagged_with_its_own_instance_in_a_shared_account() {
        let held = shared_account_kms(Some("prod"))
            .held_keys(Utc::now())
            .await
            .unwrap();
        assert_eq!(
            held,
            vec![CustodyKey {
                workspace_key_id: "0190a000-0000-7000-8000-000000000001".parse().unwrap(),
                wrapping_key_id: format!("{AWS_KMS_PREFIX}arn:aws:kms:us-east-1:1:key/k-prod"),
            }],
            "staging's, undescribable, untagged and foreign keys are never listed"
        );
    }

    #[tokio::test]
    async fn an_instance_with_no_name_lists_no_keys_at_all() {
        assert!(shared_account_kms(None)
            .held_keys(Utc::now())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn an_instance_name_that_differs_only_by_case_or_prefix_lists_nothing() {
        for name in ["Prod", "pro", "prod ", "stag"] {
            assert!(
                shared_account_kms(Some(name))
                    .held_keys(Utc::now())
                    .await
                    .unwrap()
                    .is_empty(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn kms_codes_sort_into_refusals_and_retries() {
        for code in KMS_REFUSALS {
            assert_eq!(kms_error_kind(Some(code)), SealError::Refused, "{code}");
        }
        for code in [
            Some("ThrottlingException"),
            Some("KMSInternalException"),
            Some("DependencyTimeoutException"),
            Some("KeyUnavailableException"),
            None,
        ] {
            assert_eq!(kms_error_kind(code), SealError::Unavailable, "{code:?}");
        }
    }
}
