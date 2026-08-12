//! In-memory event bus backed by a tokio broadcast channel.
//!
//! Single-process. The broadcast channel is bounded; slow subscribers
//! that fall behind by more than the channel capacity see [`Lagged`]
//! events get dropped from their view but the stream stays open. The
//! [`InMemoryBus::with_capacity`] constructor tunes the bound.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use maidan_types::{BusEnvelope, EventFilter};

use crate::item::BusItem;
use crate::sharded::ShardedBroadcast;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};

use crate::error::BusError;
use crate::stream::EventStream;
use crate::traits::EventBus;

#[derive(Debug, Clone)]
pub struct InMemoryBus {
    // Cluster 201: workspace-sharded fan-out — a publish reaches only the
    // subscribers that could match it, not every subscriber.
    fanout: Arc<ShardedBroadcast>,
}

impl InMemoryBus {
    pub fn new() -> Self {
        Self::with_capacity(crate::broadcast_cap_from_env())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            fanout: Arc::new(ShardedBroadcast::new(capacity)),
        }
    }
}

impl Default for InMemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventBus for InMemoryBus {
    async fn publish(&self, envelope: BusEnvelope) -> Result<(), BusError> {
        // Fire-and-forget: a shard with no receivers simply drops the event.
        self.fanout.publish(envelope);
        Ok(())
    }

    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream, BusError> {
        let rx = self.fanout.subscribe(&filter);
        let stream = BroadcastStream::new(rx).filter_map(move |msg| {
            let filter = filter.clone();
            async move {
                match msg {
                    Ok(envelope) if filter.matches_envelope(&envelope) => {
                        Some(BusItem::Event(Box::new(envelope)))
                    }
                    Ok(_) => None,
                    Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "inmem bus subscriber lagged");
                        Some(BusItem::Lagged { skipped })
                    }
                }
            }
        });
        Ok(Box::pin(stream))
    }
}
