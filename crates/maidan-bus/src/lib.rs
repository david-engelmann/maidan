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
pub use stream::EventStream;
pub use traits::EventBus;
