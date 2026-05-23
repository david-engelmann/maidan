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
pub mod inmem;
pub mod postgres;
pub mod stream;
pub mod traits;

pub use error::BusError;
pub use inmem::InMemoryBus;
pub use postgres::PostgresBus;
pub use stream::EventStream;
pub use traits::EventBus;
