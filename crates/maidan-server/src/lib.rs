//! Maidan HTTP server: configuration, app state, and route wiring.
//!
//! The binary entrypoint lives in `src/main.rs`; this lib crate exists so
//! integration tests can build the same router in-process and exercise
//! it through `reqwest` against a real listener.

pub mod a2a_agent;
pub mod app;
pub mod app_oauth;
pub mod apps;
pub mod auth;
pub mod bootstrap;
pub mod config;
pub mod delivery;
pub mod dm;
pub mod dto;
pub mod error;
pub mod event_stream;
pub mod federation;
pub mod federation_worker;
pub mod fsm_hook_worker;
pub mod fsm_hooks;
pub mod health;
pub mod mcp;
pub mod mcp_notifications;
pub mod mcp_quota;
pub mod mcp_stream;
pub mod mcp_streamable;
pub mod metrics;
pub mod oidc;
pub mod openapi;
pub mod outbox_relay;
pub mod presence;
pub mod quota;
pub mod rate_limit;
pub mod request_id;
pub mod routes;
pub mod session;
pub mod slash_commands;
pub mod state;
pub mod subscribe_metrics;
pub mod subscribe_resume;
pub mod thread_context;
pub mod webhook_worker;
pub mod webhooks;
pub mod ws;

pub use app::router;
pub use config::Config;
pub use state::{AppState, FederationRuntime, FsmHookRuntime, SlashRuntime, WebhookRuntime};

/// Build the maidan-server git/build version string. Falls back to the
/// crate version if no `MAIDAN_VERSION` is baked in at compile time.
pub fn version() -> &'static str {
    option_env!("MAIDAN_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}
