//! AWS KMS custody for workspace keys ([`maidan_types::WorkspaceKeyCustody`]).
//!
//! Each workspace key gets a KMS key of its own (`CreateKey`, tagged with its
//! workspace), which wraps the workspace key's 32 bytes (`Encrypt`, with the
//! workspace key's id as encryption context). The row keeps the KMS key's ARN
//! and the ciphertext, so the database alone opens nothing. Destroying a
//! workspace key schedules its KMS key's deletion, after which no copy of the
//! row, backups included, opens again.
//!
//! A KMS key per workspace key costs a monthly fee per key; that is the
//! price of an erasure that reaches backups.

use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::{KeySpec, KeyUsageType, Tag};
use maidan_types::{ContentKey, CustodyFuture, SealError, WorkspaceKeyCustody, WrappedKey};
use uuid::Uuid;
use zeroize::Zeroizing;

/// The `custody` the rows name, and the prefix of their `wrapping_key_id`.
pub const NAME: &str = "aws-kms";

/// The encryption-context key that binds a wrapped workspace key to its id.
const CONTEXT_KEY: &str = "maidan:workspace_key";

/// The tag that names a KMS key's workspace.
const WORKSPACE_TAG: &str = "maidan:workspace";

/// The deletion window KMS allows, in days.
pub const DELETION_DAYS: std::ops::RangeInclusive<i32> = 7..=30;

#[derive(Debug, Clone)]
pub struct AwsKmsCustody {
    client: aws_sdk_kms::Client,
    deletion_days: i32,
}

impl AwsKmsCustody {
    /// A custody on `client` that schedules a destroyed key's deletion
    /// `deletion_days` out (clamped to what KMS allows).
    pub fn new(client: aws_sdk_kms::Client, deletion_days: i32) -> Self {
        Self {
            client,
            deletion_days: deletion_days.clamp(*DELETION_DAYS.start(), *DELETION_DAYS.end()),
        }
    }

    fn arn(kek_id: &str) -> Result<&str, SealError> {
        kek_id
            .strip_prefix(NAME)
            .and_then(|rest| rest.strip_prefix(':'))
            .filter(|arn| !arn.is_empty())
            .ok_or_else(|| SealError::UnknownKek(kek_id.to_string()))
    }

    fn failed(operation: &str, err: impl std::fmt::Display) -> SealError {
        SealError::Custody(format!("KMS {operation}: {err}"))
    }

    async fn schedule_deletion(&self, arn: &str) -> Result<(), SealError> {
        match self
            .client
            .schedule_key_deletion()
            .key_id(arn)
            .pending_window_in_days(self.deletion_days)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(err) => {
                let service = err.as_service_error();
                // Already on its way to deletion, or gone: destroyed is what
                // was asked.
                if service.is_some_and(|e| {
                    e.is_kms_invalid_state_exception() || e.is_not_found_exception()
                }) {
                    Ok(())
                } else {
                    Err(Self::failed(
                        "ScheduleKeyDeletion",
                        aws_sdk_kms::error::DisplayErrorContext(err),
                    ))
                }
            }
        }
    }
}

impl WorkspaceKeyCustody for AwsKmsCustody {
    fn name(&self) -> &'static str {
        NAME
    }

    fn wrap<'a>(
        &'a self,
        workspace_key_id: Uuid,
        workspace_id: Uuid,
        key: &'a ContentKey,
    ) -> CustodyFuture<'a, WrappedKey> {
        Box::pin(async move {
            let tag = Tag::builder()
                .tag_key(WORKSPACE_TAG)
                .tag_value(workspace_id.to_string())
                .build()
                .map_err(|err| Self::failed("CreateKey", err))?;
            let created = self
                .client
                .create_key()
                .description(format!("Maidan workspace key {workspace_key_id}"))
                .key_usage(KeyUsageType::EncryptDecrypt)
                .key_spec(KeySpec::SymmetricDefault)
                .tags(tag)
                .send()
                .await
                .map_err(|err| {
                    Self::failed("CreateKey", aws_sdk_kms::error::DisplayErrorContext(err))
                })?;
            let arn = created
                .key_metadata()
                .and_then(|meta| meta.arn().or(Some(meta.key_id())))
                .filter(|arn| !arn.is_empty())
                .ok_or_else(|| Self::failed("CreateKey", "no key id in the answer"))?
                .to_string();
            let encrypted = self
                .client
                .encrypt()
                .key_id(&arn)
                .plaintext(Blob::new(key.as_bytes().to_vec()))
                .encryption_context(CONTEXT_KEY, workspace_key_id.to_string())
                .send()
                .await;
            let blob = match encrypted
                .map_err(|err| {
                    Self::failed("Encrypt", aws_sdk_kms::error::DisplayErrorContext(err))
                })
                .and_then(|out| {
                    out.ciphertext_blob()
                        .map(|blob| blob.as_ref().to_vec())
                        .ok_or_else(|| Self::failed("Encrypt", "no ciphertext in the answer"))
                }) {
                Ok(blob) => blob,
                Err(err) => {
                    // The key made for it wraps nothing; it goes rather than
                    // cost a fee for nothing.
                    if let Err(cleanup) = self.schedule_deletion(&arn).await {
                        tracing::warn!(error = %cleanup, "KMS key made for a failed wrap not scheduled for deletion");
                    }
                    return Err(err);
                }
            };
            Ok(WrappedKey {
                kek_id: format!("{NAME}:{arn}"),
                blob,
            })
        })
    }

    fn unwrap<'a>(
        &'a self,
        workspace_key_id: Uuid,
        wrapped: &'a WrappedKey,
    ) -> CustodyFuture<'a, ContentKey> {
        Box::pin(async move {
            let arn = Self::arn(&wrapped.kek_id)?;
            let out = self
                .client
                .decrypt()
                .key_id(arn)
                .ciphertext_blob(Blob::new(wrapped.blob.clone()))
                .encryption_context(CONTEXT_KEY, workspace_key_id.to_string())
                .send()
                .await
                .map_err(|err| {
                    Self::failed("Decrypt", aws_sdk_kms::error::DisplayErrorContext(err))
                })?;
            let plaintext = Zeroizing::new(
                out.plaintext()
                    .map(|blob| blob.as_ref().to_vec())
                    .ok_or_else(|| Self::failed("Decrypt", "no plaintext in the answer"))?,
            );
            let bytes: [u8; 32] = plaintext
                .as_slice()
                .try_into()
                .map_err(|_| SealError::Malformed)?;
            Ok(ContentKey::from_bytes(bytes))
        })
    }

    fn destroy<'a>(&'a self, wrapped: &'a WrappedKey) -> CustodyFuture<'a, ()> {
        Box::pin(async move { self.schedule_deletion(Self::arn(&wrapped.kek_id)?).await })
    }
}
