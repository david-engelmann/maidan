use async_trait::async_trait;
use maidan_types::{Event, EventFilter};

use crate::error::BusError;
use crate::stream::EventStream;

/// Backend-agnostic pub/sub interface for Maidan events.
///
/// Subscribers receive only events matching their [`EventFilter`].
/// Implementations must enforce filter matching client-side; backends
/// that broadcast all events still keep the filter logic on the
/// subscriber side so behavior is identical across impls.
#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publish an event to all matching subscribers. Returns once the
    /// event has been accepted by the backend (not once every subscriber
    /// has consumed it).
    async fn publish(&self, event: Event) -> Result<(), BusError>;

    /// Subscribe with the given filter. Returns a stream of matching
    /// events for the lifetime of the subscription.
    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream, BusError>;
}
