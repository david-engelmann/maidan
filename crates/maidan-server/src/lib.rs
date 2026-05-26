//! Maidan HTTP server: configuration, app state, and route wiring.
//!
//! The binary entrypoint lives in `src/main.rs`; this lib crate exists so
//! integration tests can build the same router in-process and exercise
//! it through `reqwest` against a real listener.

pub mod app;
pub mod auth;
pub mod bootstrap;
pub mod config;
pub mod dto;
pub mod error;
pub mod event_stream;
pub mod federation;
pub mod federation_worker;
pub mod health;
pub mod mcp;
pub mod mcp_stream;
pub mod metrics;
pub mod openapi;
pub mod request_id;
pub mod routes;
pub mod state;
pub mod ws;

pub use app::router;
pub use config::Config;
pub use state::{AppState, FederationRuntime};

/// Build the maidan-server git/build version string. Falls back to the
/// crate version if no `MAIDAN_VERSION` is baked in at compile time.
pub fn version() -> &'static str {
    option_env!("MAIDAN_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}
