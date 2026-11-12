//! Event bus for Maidan.
//!
//! Two implementations of [`EventBus`]:
//!
//! - [`InMemoryBus`] — in-process tokio broadcast channel. Default for
//!   single-process deployments (SQLite dev, tests).
//! - [`PostgresBus`] — Postgres `LISTEN`/`NOTIFY` for multi-process
//!   fan-out. The notification payload is the full serialized event;
//!   payloads larger than the Postgres NOTIFY limit (~7.9 KB) are
//!   rejected with [`BusError::PayloadTooLarge`].

#![forbid(unsafe_code)]

pub mod error;
pub mod hydrate_stats;
pub mod inmem;
pub mod item;
pub mod listener_health;
mod notify_floor;
pub mod postgres;
pub mod presence_notify;
pub mod resource_notify;
pub mod sharded;
pub mod stream;
mod sync;
/// Test doubles for integration tests in downstream crates.
pub mod test_support;
pub mod traits;

pub use error::BusError;
pub use hydrate_stats::{HydrateResult, HydrateSnapshot, HydrateStats};
pub use inmem::InMemoryBus;
pub use item::BusItem;
pub use listener_health::ListenerHealth;
pub use postgres::{PostgresBus, PostgresBusOptions};
pub use presence_notify::{
    InMemoryPresenceNotifier, PostgresPresenceNotifier, PresenceEvent, PresenceEventKind,
    PresenceNotifier,
};
pub use resource_notify::{
    InMemoryResourceNotifier, PostgresResourceNotifier, ResourceNotifier, ResourceUpdate,
};
pub use stream::EventStream;
pub use traits::EventBus;

/// Default capacity for the process-local broadcast channels that back the
/// event bus and the presence/resource notifiers.
pub const DEFAULT_BROADCAST_CAP: usize = 1024;

/// The most slots one broadcast channel may hold. Larger values panic inside
/// tokio or pre-allocate a huge buffer per workspace shard.
pub const MAX_BROADCAST_CAP: usize = 65_536;

pub(crate) fn bounded_broadcast_capacity(capacity: usize) -> usize {
    capacity.clamp(1, MAX_BROADCAST_CAP)
}

/// Aborts background listener tasks when the last bus or notifier clone drops.
pub(crate) struct AbortOnDrop {
    handles: Vec<tokio::task::AbortHandle>,
}

impl AbortOnDrop {
    pub(crate) fn new(handles: Vec<tokio::task::AbortHandle>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { handles })
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for handle in &self.handles {
            handle.abort();
        }
    }
}

/// Resolve the broadcast-channel capacity from `MAIDAN_BUS_BROADCAST_CAP`,
/// falling back to [`DEFAULT_BROADCAST_CAP`]. A larger cap lets a slow
/// subscriber lag further before the broadcast channel drops the oldest frames
/// (`RecvError::Lagged`), at the cost of more retained memory per channel.
/// Non-positive or unparseable values fall back to the default.
pub fn broadcast_cap_from_env() -> usize {
    broadcast_cap(std::env::var("MAIDAN_BUS_BROADCAST_CAP").ok().as_deref())
}

/// The capacity a raw `MAIDAN_BUS_BROADCAST_CAP` value asks for, or the
/// default when it is unset, zero or not a number.
fn broadcast_cap(raw: Option<&str>) -> usize {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_BROADCAST_CAP)
        .min(MAX_BROADCAST_CAP)
}

#[cfg(test)]
mod broadcast_cap_tests {
    use super::*;

    #[test]
    fn a_positive_cap_is_used_as_given() {
        assert_eq!(broadcast_cap(Some("5")), 5);
        assert_eq!(broadcast_cap(Some("4096")), 4096);
    }

    #[test]
    fn a_zero_missing_or_unparseable_cap_falls_back_to_the_default() {
        assert_eq!(broadcast_cap(Some("0")), DEFAULT_BROADCAST_CAP);
        assert_eq!(broadcast_cap(None), DEFAULT_BROADCAST_CAP);
        assert_eq!(broadcast_cap(Some("-3")), DEFAULT_BROADCAST_CAP);
        assert_eq!(broadcast_cap(Some("lots")), DEFAULT_BROADCAST_CAP);
        assert_eq!(broadcast_cap(Some("  ")), DEFAULT_BROADCAST_CAP);
    }

    #[test]
    fn whitespace_is_trimmed_and_a_huge_cap_is_clamped() {
        assert_eq!(broadcast_cap(Some("  5  ")), 5);
        assert_eq!(broadcast_cap(Some("999999999")), MAX_BROADCAST_CAP);
        assert_eq!(bounded_broadcast_capacity(0), 1);
        assert_eq!(
            bounded_broadcast_capacity(MAX_BROADCAST_CAP + 1),
            MAX_BROADCAST_CAP
        );
    }
}
