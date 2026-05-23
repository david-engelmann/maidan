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
//! endpoint (`POST /mcp`). A stdio transport for desktop clients arrives
//! in a later cluster.

pub mod error;
pub mod prompts;
pub mod protocol;
pub mod resources;
pub mod server;
pub mod tools;

pub use error::McpError;
pub use protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
pub use server::McpServer;
