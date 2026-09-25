//! Maidan HTTP server: configuration, app state, and route wiring.
//!
//! The binary entrypoint lives in `src/main.rs`; this lib crate exists so
//! integration tests can build the same router in-process and exercise
//! it through `reqwest` against a real listener.

pub mod a2a_agent;
pub mod a2a_grpc;
pub mod agui;
pub mod agui_stream;
pub mod app;
pub mod app_oauth;
pub mod apps;
pub mod audit;
pub mod auth;
pub mod automation_deliveries;
pub mod automation_delivery;
pub mod automation_worker;
#[cfg(feature = "bootstrap")]
pub mod bootstrap;
pub mod chain_verify;
pub mod config;
pub mod consistency;
pub mod delivery;
pub mod delivery_ops;
pub mod digest;
pub mod dm;
pub mod dto;
pub mod egress_body;
pub mod egress_http;
pub mod egress_worker;
pub mod embed_repair;
pub mod error;
pub mod event_stream;
pub mod export;
pub mod federation;
pub mod federation_worker;
pub mod fsm_hook_worker;
pub mod fsm_hooks;
pub mod github;
pub mod group_dm;
pub mod health;
pub mod import;
pub mod land_gate_advisor;
pub mod mail;
pub mod mail_worker;
pub mod mcp;
pub mod mcp_notifications;
pub mod mcp_quota;
pub mod mcp_stream;
pub mod mcp_streamable;
pub mod metrics;
pub mod notification_router;
pub mod oidc;
pub mod openapi;
pub mod outbox_relay;
pub mod presence;
pub mod quota;
pub mod rate_limit;
pub mod reindex_ops;
pub mod request_id;
pub mod result_delivery;
pub mod retention;
pub mod room_lsn;
pub mod routes;
pub mod scheduler;
pub mod scim;
pub mod secret_broker;
pub mod session;
pub mod share_consumer;
pub mod slack;
pub mod slash_commands;
pub mod state;
pub mod subscribe_grants;
pub mod subscribe_metrics;
pub mod subscribe_resume;
pub mod thread_context;
pub mod wait_sweeper;
pub mod wasi_handler;
pub mod web_push;
pub mod webhook_worker;
pub mod webhooks;
pub mod ws;

pub use app::router;
pub use config::Config;
pub use state::{AppState, FederationRuntime, FsmHookRuntime, SlashRuntime, WebhookRuntime};

/// How long a SIGTERM'd server keeps its listener open after readiness starts
/// failing: `MAIDAN_SHUTDOWN_DRAIN_SECS`, default 0 (the Helm chart and k8s
/// manifests set 5). Longer than the readiness probe period, shorter than the
/// pod's `terminationGracePeriodSeconds`.
pub fn shutdown_drain_from_env() -> std::time::Duration {
    std::time::Duration::from_secs(
        std::env::var("MAIDAN_SHUTDOWN_DRAIN_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            .min(120),
    )
}

/// Build the maidan-server git/build version string. Falls back to the
/// crate version if no `MAIDAN_VERSION` is baked in at compile time.
pub fn version() -> &'static str {
    option_env!("MAIDAN_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}
