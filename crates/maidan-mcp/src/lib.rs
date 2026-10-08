//! Model Context Protocol (MCP) server surface for Maidan.
//!
//! Implements a subset of the MCP JSON-RPC 2.0 spec, negotiating `2026-07-28`
//! (current; stateless Streamable HTTP + SEP-2243 routing headers) or `2024-11-05`:
//! - `initialize` handshake, and `server/discover` (the `2026-07-28` way to
//!   learn the versions, capabilities and instructions)
//! - `tools/list` + `tools/call`
//! - `resources/list` (the caller's workspace) + `resources/templates/list` + `resources/read`
//! - `resources/subscribe` + `resources/unsubscribe`, scoped to the subscribing
//!   caller and session ([`subscriptions`])
//! - `prompts/list` + `prompts/get`
//! - a `ttlMs` and `cacheScope` on every cacheable result ([`caching`])
//! - worker and reviewer tool profiles on their own endpoints ([`profiles`])
//!
//! Transport-agnostic: the [`McpServer`] takes JSON-RPC requests and
//! returns responses. `maidan-server` wraps it behind HTTP POST
//! endpoints (`POST /mcp`, `POST /mcp/worker`, `POST /mcp/reviewer`).
//! Stdio transport: [`stdio::run_stdio`] via `maidan-cli mcp-stdio`.

pub mod caching;
pub mod call_context;
pub mod claim_lease;
pub mod context;
pub mod error;
mod land_gate_advice;
pub mod profiles;
pub mod prompts;
pub mod protocol;
pub mod reference;
mod request_log;
pub mod resource_updates;
pub mod resources;
pub mod server;
pub mod slash_dispatch;
pub mod stdio;
pub mod streamable_session;
pub mod subscriptions;

pub mod tools;

pub use error::McpError;
pub use land_gate_advice::{LandGateAdviseError, LandGateAdvising};
pub use profiles::Profile;
pub use protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
pub use server::{
    is_supported_protocol_version, negotiate_protocol_version, preferred_protocol_version,
    ApprovalConfirmationKeys, McpServer, PresenceReader, DEFAULT_CONFIRMATION_TTL, INSTRUCTIONS,
    SESSION_PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
};
pub use slash_dispatch::SlashDispatcher;
pub use stdio::run_stdio;
pub use subscriptions::{McpSession, NotificationListener, Principal, Subscriber};
