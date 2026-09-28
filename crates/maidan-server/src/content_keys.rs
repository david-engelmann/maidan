//! Server wiring for crypto-shredding: the startup rewrap that finishes a KEK
//! rotation. The keyring itself comes from
//! [`maidan_store::content_keyring::from_env`].

use std::sync::Arc;
use std::time::Duration;

use maidan_store::Store;

/// Keys rewrapped per transaction by [`spawn_rewrap`].
const REWRAP_BATCH: i64 = 500;

/// Rewrap, in batches, every live content key still wrapped by a previous KEK,
/// then exit. KEKs only change at a restart, so one pass per boot completes a
/// rotation; a failed batch is retried after `retry`. Replicas running this
/// concurrently are safe: each batch re-reads under lock.
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
                Ok(0) => break,
                Ok(n) => total += n,
                Err(err) => {
                    tracing::warn!(error = %err, "content keys: rewrap batch failed; retrying");
                    tokio::time::sleep(retry).await;
                }
            }
        }
        tracing::info!(total, "content keys: rewrap complete");
    })
}
