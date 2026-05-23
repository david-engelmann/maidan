//! In-memory event bus backed by a tokio broadcast channel.
//!
//! Single-process. The broadcast channel is bounded; slow subscribers
//! that fall behind by more than the channel capacity see [`Lagged`]
//! events get dropped from their view but the stream stays open. The
//! [`InMemoryBus::with_capacity`] constructor tunes the bound.

use async_trait::async_trait;
use futures::StreamExt;
use maidan_types::{Event, EventFilter};
use tokio::sync::broadcast;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};

use crate::error::BusError;
use crate::stream::EventStream;
use crate::traits::EventBus;

#[derive(Debug, Clone)]
pub struct InMemoryBus {
    tx: broadcast::Sender<Event>,
}

impl InMemoryBus {
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }
}

impl Default for InMemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventBus for InMemoryBus {
    async fn publish(&self, event: Event) -> Result<(), BusError> {
        // `send` errors only when there are zero receivers; that is not
        // a failure for fire-and-forget pub/sub.
        let _ = self.tx.send(event);
        Ok(())
    }

    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream, BusError> {
        let rx = self.tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(move |msg| {
            let filter = filter.clone();
            async move {
                match msg {
                    Ok(event) if filter.matches(&event) => Some(event),
                    Ok(_) => None,
                    Err(BroadcastStreamRecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "inmem bus subscriber lagged");
                        None
                    }
                }
            }
        });
        Ok(Box::pin(stream))
    }
}
