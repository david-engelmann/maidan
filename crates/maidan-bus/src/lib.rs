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
    raw.and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_BROADCAST_CAP)
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
    }
}
