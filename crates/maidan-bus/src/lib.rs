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
pub mod postgres;
pub mod presence_notify;
pub mod resource_notify;
pub mod stream;
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
pub use resource_notify::{InMemoryResourceNotifier, PostgresResourceNotifier, ResourceNotifier};
pub use stream::EventStream;
pub use traits::EventBus;

/// Default capacity for the process-local broadcast channels that back the
/// event bus and the presence/resource notifiers.
pub const DEFAULT_BROADCAST_CAP: usize = 1024;

/// Resolve the broadcast-channel capacity from `MAIDAN_BUS_BROADCAST_CAP`,
/// falling back to [`DEFAULT_BROADCAST_CAP`] (Cluster 168, R1). A larger cap
/// lets a slow subscriber lag further before the broadcast channel drops the
/// oldest frames (`RecvError::Lagged`), at the cost of more retained memory per
/// channel. Non-positive or unparseable values fall back to the default.
pub fn broadcast_cap_from_env() -> usize {
    std::env::var("MAIDAN_BUS_BROADCAST_CAP")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_BROADCAST_CAP)
}
