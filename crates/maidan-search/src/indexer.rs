//! Background indexer: subscribes to the event bus and ensures every
//! state-changing message event keeps the search indexes current.
//!
//! For v0.2.0 the FT triggers (Postgres) and FTS5 triggers (SQLite)
//! already maintain the lexical index synchronously. The indexer is
//! the async pipeline that future clusters will use to generate
//! embeddings, vector-index updates, and any other side effects that
//! shouldn't block writes.
//!
//! The default [`LoggingHandler`] just observes events for metrics +
//! tracing; tests can swap in any [`EventHandler`].

use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use maidan_bus::{EventBus, EventStream};
use maidan_types::{Event, EventFilter, EventKind};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

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
}

impl Indexer {
    pub fn new(bus: Arc<dyn EventBus>, handler: Arc<dyn EventHandler>) -> Self {
        Self { bus, handler }
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
        let join = tokio::spawn(async move {
            let mut backoff = RECONNECT_INITIAL;
            loop {
                let filter = EventFilter::all()
                    .with_kinds([EventKind::MessagePosted, EventKind::MessageTombstoned]);
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
                let outcome =
                    consume(stream, self.handler.as_ref(), &mut shutdown_rx, &heartbeat).await;
                match outcome {
                    ConsumeOutcome::ShutdownRequested => return,
                    ConsumeOutcome::StreamEnded => {
                        warn!("indexer stream ended; resubscribing");
                    }
                }
            }
        });
        IndexerHandle {
            shutdown: shutdown_tx,
            join,
            last_event_unix_ms,
        }
    }
}

/// Outcome of one consume() invocation. The outer loop in `spawn` uses
/// this to decide whether to resubscribe or exit cleanly.
enum ConsumeOutcome {
    ShutdownRequested,
    StreamEnded,
}

async fn consume(
    mut stream: EventStream,
    handler: &dyn EventHandler,
    shutdown_rx: &mut mpsc::Receiver<()>,
    last_event_unix_ms: &AtomicI64,
) -> ConsumeOutcome {
    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(maidan_bus::BusItem::Event(envelope)) => {
                        handler.handle(&envelope.event).await;
                        last_event_unix_ms.store(
                            chrono::Utc::now().timestamp_millis(),
                            Ordering::Relaxed,
                        );
                    }
                    Some(maidan_bus::BusItem::Lagged { skipped }) => {
                        warn!(skipped, "indexer bus subscriber lagged; events may be missing from the index");
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
}

impl IndexerHandle {
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(()).await;
        if let Err(err) = self.join.await {
            error!(error = %err, "indexer task join failed");
        }
    }
}
