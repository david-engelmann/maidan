//! Model Context Protocol (MCP) server surface for Maidan.
//!
//! Implements a subset of the MCP JSON-RPC 2.0 spec, negotiating `2026-07-28`
//! (current; stateless Streamable HTTP + SEP-2243 routing headers) or `2024-11-05`:
//! - `initialize` handshake
//! - `tools/list` + `tools/call`
//! - `resources/list` + `resources/read`
//! - `prompts/list` + `prompts/get`
//!
//! Transport-agnostic: the [`McpServer`] takes JSON-RPC requests and
//! returns responses. `maidan-server` wraps it behind an HTTP POST
//! endpoint (`POST /mcp`). Stdio transport: [`stdio::run_stdio`] via
//! `maidan-cli mcp-stdio`.

pub mod context;
pub mod error;
pub mod prompts;
pub mod protocol;
pub mod reference;
pub mod resource_updates;
pub mod resources;
pub mod server;
pub mod stdio;
pub mod streamable_session;
pub mod tools;

pub use error::McpError;
pub use protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
pub use server::{
    is_supported_protocol_version, preferred_protocol_version, McpServer,
    SUPPORTED_PROTOCOL_VERSIONS,
};
pub use stdio::run_stdio;
