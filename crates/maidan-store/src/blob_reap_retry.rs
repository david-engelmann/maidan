//! Orphaned artifact bytes whose delete failed get another try.
//!
//! An erase, a purge, a media expiry or a refused upload can leave bytes no
//! artifact row holds. They are deleted at once under the reap lease
//! ([`crate::store::ArtifactMetaStore::reap_artifact_blob`]), but a blob store
//! that errors or stalls left them behind for good, with only a log line to
//! say so. Now a failed delete queues the sha, and the retention sweep retries
//! it through the same reap, so the lease and the check that no row holds the
//! sha apply to every try.

use maidan_types::WorkspaceId;

use crate::{BlobDelete, BlobReap, Store, StoreError};

/// How long a sha waits after its first failed retry. Each further failure
/// doubles it, up to [`MAX_BACKOFF_SECS`].
const FIRST_BACKOFF_SECS: i64 = 60;

/// A blob store that is down for a day gets a try every few hours, not one a
/// minute.
const MAX_BACKOFF_SECS: i64 = 6 * 60 * 60;

/// How long to wait after `failed_attempts` failed retries.
pub fn backoff_secs(failed_attempts: i32) -> i64 {
    let doublings = u32::try_from(failed_attempts.saturating_sub(1).clamp(0, 16)).unwrap_or(0);
    FIRST_BACKOFF_SECS
        .saturating_mul(1_i64 << doublings)
        .min(MAX_BACKOFF_SECS)
}

/// Reap `sha256`'s bytes, and queue them for another try when the delete
/// fails or overruns, or the reap itself errors. `workspace_id` is the
/// workspace whose erase, purge, expiry or refused upload orphaned them. A
/// reap that errored reads as [`BlobReap::DeleteFailed`].
pub async fn reap_or_queue(
    store: &dyn Store,
    sha256: &str,
    workspace_id: WorkspaceId,
    delete: BlobDelete<'_>,
) -> BlobReap {
    let reaped = match store.reap_artifact_blob(sha256, delete).await {
        Ok(reaped) => reaped,
        Err(err) => BlobReap::DeleteFailed(err.to_string()),
    };
    if let BlobReap::DeleteFailed(err) = &reaped {
        if let Err(queue_err) = store.queue_blob_reap(sha256, workspace_id, err).await {
            // Nothing else will find these bytes, so say so loudly.
            tracing::error!(sha = %sha256, error = %queue_err, "artifact.blob_reap_not_queued");
        }
    }
    reaped
}

/// What one [`retry_due`] pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetryTally {
    /// Bytes deleted, and their shas taken off the queue.
    pub deleted: u64,
    /// Shas an artifact row holds again, taken off the queue with their bytes
    /// kept.
    pub referenced: u64,
    /// Shas whose delete failed again, left queued for a later try.
    pub failed: u64,
}

/// Retry up to `limit` queued shas whose next try is due. `delete_for`
/// builds the blob delete for a sha, or `None` for one no blob store could
/// hold, which is dropped.
///
/// Each try is a full reap, so a sha that another workspace has uploaded
/// since it was queued comes back [`BlobReap::Referenced`]: its bytes are that
/// workspace's now and stay, and the sha leaves the queue, since that
/// workspace's own erase will reap them.
pub async fn retry_due<F>(
    store: &dyn Store,
    limit: i64,
    delete_for: F,
) -> Result<RetryTally, StoreError>
where
    F: Fn(&str) -> Option<BlobDelete<'static>>,
{
    let mut tally = RetryTally::default();
    for pending in store.due_blob_reaps(limit).await? {
        let Some(delete) = delete_for(&pending.sha256) else {
            store.settle_blob_reap(&pending.sha256).await?;
            continue;
        };
        let failed = match store.reap_artifact_blob(&pending.sha256, delete).await {
            Ok(BlobReap::Deleted) => {
                tally.deleted += 1;
                None
            }
            Ok(BlobReap::Referenced) => {
                tally.referenced += 1;
                None
            }
            Ok(BlobReap::DeleteFailed(err)) => Some(err),
            Err(err) => Some(err.to_string()),
        };
        match failed {
            None => store.settle_blob_reap(&pending.sha256).await?,
            Some(err) => {
                tally.failed += 1;
                let delay = backoff_secs(pending.attempts.saturating_add(1));
                store.defer_blob_reap(&pending.sha256, &err, delay).await?;
            }
        }
    }
    Ok(tally)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_a_minute_and_stops_at_six_hours() {
        assert_eq!(backoff_secs(1), 60);
        assert_eq!(backoff_secs(2), 120);
        assert_eq!(backoff_secs(3), 240);
        assert_eq!(backoff_secs(10), MAX_BACKOFF_SECS);
        assert_eq!(backoff_secs(i32::MAX), MAX_BACKOFF_SECS);
    }
}
