//! Server wiring for crypto-shredding: the startup rewrap that finishes a KEK
//! rotation. The keyring itself comes from
//! [`maidan_store::content_keyring::from_env`].

use std::sync::Arc;
use std::time::Duration;

use maidan_store::Store;

/// Rows per transaction in each [`spawn_rewrap`] pass.
const REWRAP_BATCH: i64 = 500;

/// Rewrap every live content key still wrapped by a previous KEK, then exit.
/// KEKs only change at a restart, so passes at boot complete a rotation. A
/// failed pass, or one left only with rows the custody refuses (counted in
/// `maidan_content_keys_refused`), is retried after `retry`; the task exits
/// only when nothing is left. Replicas running this concurrently are safe:
/// each page re-reads under lock.
pub fn spawn_rewrap(store: Arc<dyn Store>, retry: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        match store.content_keys_needing_rewrap().await {
            Ok(0) => return,
            Ok(pending) => {
                tracing::info!(pending, "content keys: rewrapping under the primary KEK")
            }
            Err(err) => tracing::warn!(error = %err, "content keys: rewrap count failed"),
        }
        let mut total = 0u64;
        loop {
            match store.rewrap_content_keys(REWRAP_BATCH).await {
                Ok(pass) => {
                    total += pass.changed;
                    metrics::gauge!("maidan_content_keys_refused").set(pass.skipped as f64);
                    if pass.skipped == 0 {
                        if pass.changed == 0 {
                            break;
                        }
                    } else if pass.changed == 0 {
                        // Only rows the custody refuses are left; they stay
                        // on their old keys until it allows them.
                        tracing::warn!(
                            refused = pass.skipped,
                            "content keys: rewrap stalled on rows the custody refuses; retrying"
                        );
                        tokio::time::sleep(retry).await;
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "content keys: rewrap pass failed; retrying");
                    tokio::time::sleep(retry).await;
                }
            }
        }
        tracing::info!(total, "content keys: rewrap complete");
    })
}
