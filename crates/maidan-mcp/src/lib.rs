//! Model Context Protocol (MCP) server surface for Maidan.
//!
//! Implements a subset of the MCP 2024-11-05 JSON-RPC 2.0 spec:
//! - `initialize` handshake
//! - `tools/list` + `tools/call`
//! - `resources/list` + `resources/read`
//! - `prompts/list` + `prompts/get`
//!
//! Transport-agnostic: the [`McpServer`] takes JSON-RPC requests and
//! returns responses. `maidan-server` wraps it behind an HTTP POST
//! endpoint (`POST /mcp`). Stdio transport: [`stdio::run_stdio`] via
//! `maidan-cli mcp-stdio`.

pub mod error;
pub mod prompts;
pub mod protocol;
pub mod reference;
pub mod resources;
pub mod server;
pub mod stdio;
pub mod tools;

pub use error::McpError;
pub use protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
pub use server::McpServer;
pub use stdio::run_stdio;
