//! Test doubles for [`EventBus`] (integration and unit tests).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use maidan_types::{BusEnvelope, EventFilter};

use crate::error::BusError;
use crate::traits::EventBus;
use crate::EventStream;

/// Always fails [`EventBus::publish`].
pub struct FailingBus {
    reason: String,
}

impl FailingBus {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

#[async_trait]
impl EventBus for FailingBus {
    async fn publish(&self, _envelope: BusEnvelope) -> Result<(), BusError> {
        Err(BusError::HydrateFailed {
            log_id: 0,
            reason: self.reason.clone(),
        })
    }

    async fn subscribe(&self, _filter: EventFilter) -> Result<EventStream, BusError> {
        Err(BusError::Closed)
    }
}

/// Delegates subscribe to `inner` and counts [`EventBus::publish`] calls.
pub struct RecordingBus {
    pub inner: Arc<dyn EventBus>,
    pub publish_count: Arc<AtomicUsize>,
}

impl RecordingBus {
    pub fn new(inner: Arc<dyn EventBus>) -> Self {
        Self {
            inner,
            publish_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn publishes(&self) -> usize {
        self.publish_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl EventBus for RecordingBus {
    async fn publish(&self, envelope: BusEnvelope) -> Result<(), BusError> {
        self.publish_count.fetch_add(1, Ordering::Relaxed);
        self.inner.publish(envelope).await
    }

    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream, BusError> {
        self.inner.subscribe(filter).await
    }
}
