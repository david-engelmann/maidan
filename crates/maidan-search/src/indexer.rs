//! Background indexer: a **tap projector** over the event log (Cluster 393).
//!
//! Lexical FT/FTS5 triggers still maintain the index on write. This task
//! is the async projection (embeddings and any other side effect that
//! must not block the append). It is bound by the tap contract: verify
//! every backfill page, drain history before live, filter to message
//! kinds, fail closed on a gap or chain break. A silently diverged
//! index is a bug.
//!
//! The default [`LoggingHandler`] just observes events for metrics +
//! tracing; tests can swap in any [`EventHandler`].

use std::{
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use maidan_bus::{EventBus, EventStream};
use maidan_store::Store;
use maidan_types::{Event, EventFilter, EventKind, SEARCH_PROJECTOR_KINDS};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

use crate::tap_projector::{backfill_search, SearchTap};

/// Per-event behavior. Implementations should be cheap and non-blocking;
/// the indexer awaits them serially within a single subscription.
#[async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &Event);
}

/// Logging-only handler. Useful in development and as a baseline test
/// double. Records every event observed in a shared `Vec`; tests poll
/// the log with [`LoggingHandler::wait_for`] rather than racing a
/// `Notify` (which only wakes current waiters).
#[derive(Debug, Default)]
pub struct LoggingHandler {
    pub observed: tokio::sync::Mutex<Vec<EventKind>>,
}

impl LoggingHandler {
    /// Block up to `timeout` until `predicate(log)` returns true; return
    /// a snapshot of the log at the point the predicate flipped, or
    /// `None` on timeout.
    pub async fn wait_for<F>(&self, timeout: Duration, predicate: F) -> Option<Vec<EventKind>>
    where
        F: Fn(&[EventKind]) -> bool,
    {
        tokio::time::timeout(timeout, async {
            loop {
                {
                    let log = self.observed.lock().await;
                    if predicate(&log) {
                        return log.clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .ok()
    }
}

#[async_trait]
impl EventHandler for LoggingHandler {
    async fn handle(&self, event: &Event) {
        let kind = event.kind();
        debug!(?kind, "indexer observed event");
        let mut log = self.observed.lock().await;
        log.push(kind);
    }
}

/// Backoff parameters for transient bus errors.
const RECONNECT_INITIAL: Duration = Duration::from_millis(100);
const RECONNECT_MAX: Duration = Duration::from_secs(5);

/// Long-running indexer task. Spawn with [`Indexer::spawn`] from
/// `maidan-server` startup; abort via the returned [`IndexerHandle`] on
/// shutdown.
pub struct Indexer {
    bus: Arc<dyn EventBus>,
    handler: Arc<dyn EventHandler>,
    /// Durable log for `BusItem::Lagged` resume (Cluster 388). Absent in
    /// unit tests that only drive the live bus.
    log: Option<Arc<dyn Store>>,
}

impl Indexer {
    pub fn new(bus: Arc<dyn EventBus>, handler: Arc<dyn EventHandler>) -> Self {
        Self {
            bus,
            handler,
            log: None,
        }
    }

    pub fn with_log(mut self, log: Arc<dyn Store>) -> Self {
        self.log = Some(log);
        self
    }

    /// Spawn the indexer as a tokio task. The returned handle owns the
    /// `JoinHandle` and a shutdown signal; dropping it aborts the task.
    pub fn spawn(self) -> IndexerHandle {
        self.spawn_with_heartbeat(Arc::new(AtomicI64::new(0)))
    }

    /// Like [`spawn`](Self::spawn) but exposes `last_event_unix_ms` for health probes.
    pub fn spawn_with_heartbeat(self, last_event_unix_ms: Arc<AtomicI64>) -> IndexerHandle {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let heartbeat = last_event_unix_ms.clone();
        let rebuild_needed = Arc::new(AtomicBool::new(false));
        let rebuild_flag = rebuild_needed.clone();
        let join = tokio::spawn(async move {
            let mut backoff = RECONNECT_INITIAL;
            loop {
                let filter = EventFilter::all().with_kinds(SEARCH_PROJECTOR_KINDS.iter().copied());
                let stream = match self.bus.subscribe(filter).await {
                    Ok(s) => s,
                    Err(err) => {
                        warn!(error = %err, ?backoff, "indexer bus subscribe failed; retrying");
                        if tokio::time::timeout(backoff, shutdown_rx.recv())
                            .await
                            .is_ok()
                        {
                            return;
                        }
                        backoff = (backoff * 2).min(RECONNECT_MAX);
                        continue;
                    }
                };
                backoff = RECONNECT_INITIAL;
                info!("indexer attached to bus");
                let outcome = consume(
                    stream,
                    self.handler.as_ref(),
                    self.log.as_deref(),
                    &mut shutdown_rx,
                    &heartbeat,
                    &rebuild_flag,
                )
                .await;
                match outcome {
                    ConsumeOutcome::ShutdownRequested => return,
                    ConsumeOutcome::StreamEnded => {
                        warn!("indexer stream ended; resubscribing");
                    }
                    ConsumeOutcome::RebuildRequired => {
                        rebuild_flag.store(true, Ordering::Relaxed);
                        error!("search projector must rebuild; index must not diverge silently");
                        if tokio::time::timeout(backoff, shutdown_rx.recv())
                            .await
                            .is_ok()
                        {
                            return;
                        }
                        backoff = (backoff * 2).min(RECONNECT_MAX);
                    }
                }
            }
        });
        IndexerHandle {
            shutdown: shutdown_tx,
            join,
            last_event_unix_ms,
            rebuild_needed,
        }
    }
}

/// Outcome of one consume() invocation. The outer loop in `spawn` uses
/// this to decide whether to resubscribe or exit cleanly.
enum ConsumeOutcome {
    ShutdownRequested,
    StreamEnded,
    RebuildRequired,
}

async fn project_row(
    handler: &dyn EventHandler,
    row: maidan_types::StoredEvent,
) -> Result<(), maidan_types::TapFault> {
    let event = serde_json::from_value::<Event>(row.payload)
        .map_err(|_| maidan_types::TapFault::MissingHistory)?;
    handler.handle(&event).await;
    Ok(())
}

async fn consume(
    mut stream: EventStream,
    handler: &dyn EventHandler,
    log: Option<&dyn Store>,
    shutdown_rx: &mut mpsc::Receiver<()>,
    last_event_unix_ms: &AtomicI64,
    rebuild_flag: &AtomicBool,
) -> ConsumeOutcome {
    let mut tap = SearchTap::new();
    let mut watermark: i64 = 0;
    if let Some(store) = log {
        match backfill_search(store, &mut tap, |row| async move {
            project_row(handler, row).await
        })
        .await
        {
            Ok(hw) => {
                watermark = hw;
                rebuild_flag.store(false, Ordering::Relaxed);
            }
            Err(fault) => {
                error!(?fault, "search projector backfill failed closed");
                rebuild_flag.store(true, Ordering::Relaxed);
                return ConsumeOutcome::RebuildRequired;
            }
        }
    }

    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(maidan_bus::BusItem::Event(envelope)) => {
                        if log.is_some() && envelope.log_id <= watermark {
                            continue;
                        }
                        watermark = watermark.max(envelope.log_id);
                        handler.handle(&envelope.event).await;
                        last_event_unix_ms.store(
                            chrono::Utc::now().timestamp_millis(),
                            Ordering::Relaxed,
                        );
                    }
                    Some(maidan_bus::BusItem::Lagged { skipped }) => {
                        let Some(store) = log else {
                            error!(
                                skipped,
                                "indexer lagged with no durable log; search projector must rebuild"
                            );
                            rebuild_flag.store(true, Ordering::Relaxed);
                            return ConsumeOutcome::RebuildRequired;
                        };
                        warn!(
                            skipped,
                            watermark, "indexer bus subscriber lagged; re-verifying log"
                        );
                        tap = SearchTap::new();
                        match backfill_search(store, &mut tap, |row| async {
                            project_row(handler, row).await
                        })
                        .await
                        {
                            Ok(hw) => {
                                watermark = hw;
                                rebuild_flag.store(false, Ordering::Relaxed);
                            }
                            Err(fault) => {
                                error!(?fault, "search projector lag rebuild failed closed");
                                rebuild_flag.store(true, Ordering::Relaxed);
                                return ConsumeOutcome::RebuildRequired;
                            }
                        }
                        last_event_unix_ms.store(
                            chrono::Utc::now().timestamp_millis(),
                            Ordering::Relaxed,
                        );
                    }
                    None => return ConsumeOutcome::StreamEnded,
                }
            }
            _ = shutdown_rx.recv() => {
                info!("indexer shutdown received");
                return ConsumeOutcome::ShutdownRequested;
            }
        }
    }
}

pub struct IndexerHandle {
    shutdown: mpsc::Sender<()>,
    join: tokio::task::JoinHandle<()>,
    pub last_event_unix_ms: Arc<AtomicI64>,
    /// Set when the tap contract fails closed (gap, chain break, lagged
    /// without a log). Operators rebuild from the messages table; the
    /// indexer must not keep projecting a gapped suffix.
    pub rebuild_needed: Arc<AtomicBool>,
}

impl IndexerHandle {
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(()).await;
        if let Err(err) = self.join.await {
            error!(error = %err, "indexer task join failed");
        }
    }
}
