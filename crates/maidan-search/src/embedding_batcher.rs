//! Batching live-indexing handler (Cluster 116).
//!
//! The default [`EmbeddingHandler`](crate::EmbeddingHandler) embeds one message
//! per event, serially — one provider round-trip per message. This handler
//! instead enqueues live messages onto a **bounded** channel; a worker drains
//! them to [`EmbeddingProvider::embed_batch`] in batches:
//!
//! - **Batching:** the worker takes up to `batch_size` jobs per provider call.
//! - **Backpressure:** the channel is bounded, so when the worker falls behind,
//!   [`handle`](BatchingEmbeddingHandler::handle) awaits `send` — slowing bus
//!   consumption instead of growing memory without bound. Queue depth is tracked
//!   in [`IndexerMetrics`] and is hard-capped by the channel capacity, so the
//!   lag is both observable and bounded.
//! - **Isolation:** backfill (`reindex`) runs on its own task and never touches
//!   this queue, so a large-workspace backfill can't delay live indexing.

use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use maidan_store::Store;
use maidan_types::{Event, MessageId, ThreadState};
use tokio::sync::{mpsc, RwLock};
use tracing::warn;

use crate::embedding_provider::EmbeddingProvider;
use crate::indexer::EventHandler;
use crate::{Search, SearchError};

type HealthSlot = Option<Arc<RwLock<Option<String>>>>;

/// Shared, lock-free counters for the live embedding pipeline. The server
/// mirrors these into Prometheus gauges each scrape.
#[derive(Debug)]
pub struct IndexerMetrics {
    /// Messages enqueued but not yet embedded. Hard-capped by `queue_capacity`.
    pub queue_depth: AtomicUsize,
    /// Channel capacity — the upper bound on `queue_depth`.
    pub queue_capacity: usize,
    /// Messages successfully embedded + upserted.
    pub embedded_total: AtomicU64,
    /// Messages dropped after an embed/upsert failure.
    pub failed_total: AtomicU64,
    /// Provider batch calls issued.
    pub batches_total: AtomicU64,
}

impl IndexerMetrics {
    pub fn new(queue_capacity: usize) -> Self {
        Self {
            queue_depth: AtomicUsize::new(0),
            queue_capacity,
            embedded_total: AtomicU64::new(0),
            failed_total: AtomicU64::new(0),
            batches_total: AtomicU64::new(0),
        }
    }
}

impl Default for IndexerMetrics {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Live-indexer batching tunables (from env, with defaults).
#[derive(Debug, Clone, Copy)]
pub struct BatchConfig {
    pub queue_capacity: usize,
    pub batch_size: usize,
}

impl BatchConfig {
    pub fn from_env() -> Self {
        let queue_capacity = env_usize("MAIDAN_INDEXER_QUEUE_CAPACITY", 1024).max(1);
        let batch_size = env_usize("MAIDAN_INDEXER_BATCH_SIZE", 32).clamp(1, queue_capacity);
        Self {
            queue_capacity,
            batch_size,
        }
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

struct EmbedJob {
    message_id: MessageId,
    body: String,
}

pub struct BatchingEmbeddingHandler {
    store: Arc<dyn Store>,
    tx: mpsc::Sender<EmbedJob>,
    metrics: Arc<IndexerMetrics>,
    health_error: HealthSlot,
}

impl BatchingEmbeddingHandler {
    /// Build the handler and spawn its worker. The worker stops when the
    /// handler (and thus the sender) is dropped. `metrics.queue_capacity` must
    /// equal `config.queue_capacity`.
    pub fn spawn(
        store: Arc<dyn Store>,
        search: Arc<dyn Search>,
        provider: Arc<dyn EmbeddingProvider>,
        config: BatchConfig,
        metrics: Arc<IndexerMetrics>,
        health_error: HealthSlot,
    ) -> Self {
        let (tx, rx) = mpsc::channel(config.queue_capacity);
        tokio::spawn(run_worker(
            rx,
            search,
            provider,
            metrics.clone(),
            health_error.clone(),
            config.batch_size,
        ));
        Self {
            store,
            tx,
            metrics,
            health_error,
        }
    }

    async fn set_health_error(&self, msg: String) {
        set_health(&self.health_error, msg).await;
    }
}

async fn set_health(slot: &HealthSlot, msg: String) {
    if let Some(slot) = slot {
        *slot.write().await = Some(msg);
    }
}

async fn clear_health(slot: &HealthSlot) {
    if let Some(slot) = slot {
        *slot.write().await = None;
    }
}

#[async_trait]
impl EventHandler for BatchingEmbeddingHandler {
    async fn handle(&self, event: &Event) {
        let (thread_id, message) = match event {
            Event::MessagePosted {
                thread_id, message, ..
            }
            | Event::MessageEdited {
                thread_id, message, ..
            } => (*thread_id, message),
            _ => return,
        };

        let thread = match self.store.get_thread(thread_id).await {
            Ok(t) => t,
            Err(err) => {
                self.set_health_error(format!("load thread failed: {err}"))
                    .await;
                warn!(%err, "batch indexer: load thread failed");
                return;
            }
        };
        if thread.tombstoned_at.is_some() || thread.state == ThreadState::Archived {
            return;
        }

        // Backpressure: a full queue makes this `send` await, slowing bus
        // consumption rather than growing memory. queue_depth is incremented
        // before the send and decremented by the worker as it drains a batch.
        self.metrics.queue_depth.fetch_add(1, Ordering::Relaxed);
        let job = EmbedJob {
            message_id: message.id,
            body: message.body.clone(),
        };
        if self.tx.send(job).await.is_err() {
            self.metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
            warn!("batch indexer: embedding worker stopped; dropping live index job");
        }
    }
}

async fn run_worker(
    mut rx: mpsc::Receiver<EmbedJob>,
    search: Arc<dyn Search>,
    provider: Arc<dyn EmbeddingProvider>,
    metrics: Arc<IndexerMetrics>,
    health_error: HealthSlot,
    batch_size: usize,
) {
    let model = provider.model_name().to_string();
    while let Some(first) = rx.recv().await {
        let mut batch = vec![first];
        while batch.len() < batch_size {
            match rx.try_recv() {
                Ok(job) => batch.push(job),
                Err(_) => break,
            }
        }
        let n = batch.len();
        metrics.queue_depth.fetch_sub(n, Ordering::Relaxed);
        metrics.batches_total.fetch_add(1, Ordering::Relaxed);

        // The provider may be a blocking HTTP client; keep it off the runtime.
        let bodies: Vec<String> = batch.iter().map(|j| j.body.clone()).collect();
        let provider_cl = provider.clone();
        let embed = tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = bodies.iter().map(|s| s.as_str()).collect();
            provider_cl.embed_batch(&refs)
        })
        .await;

        let embeddings = match embed {
            Ok(Ok(v)) => v,
            Ok(Err(err)) => {
                set_health(&health_error, format!("embedding generation failed: {err}")).await;
                warn!(%err, batch = n, "batch indexer: embedding batch failed");
                metrics.failed_total.fetch_add(n as u64, Ordering::Relaxed);
                continue;
            }
            Err(join_err) => {
                warn!(%join_err, "batch indexer: embed task join failed");
                metrics.failed_total.fetch_add(n as u64, Ordering::Relaxed);
                continue;
            }
        };

        let mut any_ok = false;
        for (job, embedding) in batch.iter().zip(embeddings.iter()) {
            match search
                .upsert_embedding(job.message_id, &model, embedding)
                .await
            {
                Ok(()) => {
                    metrics.embedded_total.fetch_add(1, Ordering::Relaxed);
                    any_ok = true;
                }
                Err(SearchError::Unsupported(_)) => {}
                Err(err) => {
                    metrics.failed_total.fetch_add(1, Ordering::Relaxed);
                    set_health(&health_error, format!("embedding upsert failed: {err}")).await;
                    warn!(%err, message_id = %job.message_id, "batch indexer: upsert failed");
                }
            }
        }
        if any_ok {
            clear_health(&health_error).await;
        }
    }
}
