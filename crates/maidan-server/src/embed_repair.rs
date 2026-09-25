//! Repair of live embeddings the indexer missed.
//!
//! The live indexer retries a failed batch a few times and then gives up, and
//! a message posted while the process was stopping never reaches it at all.
//! Either way the message is searchable by text but not by meaning, and before
//! this only a full reindex brought it back. This sweep embeds, on a timer, the
//! newest live messages that still have no embedding for the active model.
//! Postgres takes an advisory lock so replicas don't repeat each other's work.
//!
//! On by default wherever embeddings are generated. `MAIDAN_EMBED_REPAIR_INTERVAL_SECS`
//! (default 300; `0` disables) and `MAIDAN_EMBED_REPAIR_BATCH` (default 256).

use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;

use maidan_search::{EmbeddingProvider, IndexerMetrics, Search, SearchError};

#[derive(Debug, Clone, Copy)]
pub struct RepairConfig {
    pub interval: Duration,
    pub batch: i64,
}

pub fn config_from_env() -> Option<RepairConfig> {
    let secs = std::env::var("MAIDAN_EMBED_REPAIR_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);
    if secs == 0 {
        return None;
    }
    let batch = std::env::var("MAIDAN_EMBED_REPAIR_BATCH")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(256)
        .clamp(1, 10_000);
    Some(RepairConfig {
        interval: Duration::from_secs(secs),
        batch,
    })
}

/// One pass. `false` when the backend cannot embed, so the caller stops.
pub async fn repair_once(
    search: &dyn Search,
    provider: &dyn EmbeddingProvider,
    metrics: &IndexerMetrics,
    batch: i64,
) -> bool {
    match search.embed_missing(provider, batch).await {
        Ok(report) => {
            metrics
                .repaired_total
                .fetch_add(report.processed, Ordering::Relaxed);
            if report.processed > 0 || report.failed > 0 {
                tracing::info!(
                    repaired = report.processed,
                    failed = report.failed,
                    "embed repair: pass complete"
                );
            }
            true
        }
        Err(SearchError::Unsupported(_)) => false,
        Err(err) => {
            tracing::warn!(error = %err, "embed repair: pass failed");
            true
        }
    }
}

pub async fn run(
    search: Arc<dyn Search>,
    provider: Arc<dyn EmbeddingProvider>,
    metrics: Arc<IndexerMetrics>,
    config: RepairConfig,
) {
    let mut ticker = tokio::time::interval(config.interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick is immediate; skip it so startup is not a repair storm.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        if !repair_once(search.as_ref(), provider.as_ref(), &metrics, config.batch).await {
            tracing::info!("embed repair: backend has no embeddings; stopping");
            return;
        }
    }
}
