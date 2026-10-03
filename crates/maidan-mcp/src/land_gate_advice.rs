//! Advisory land-gate scoring, shared with the HTTP advisor.
//!
//! The MCP tool calls this and does not write the gate. The HTTP server
//! installs the same advisor it serves on `POST /threads/{id}/land-gate/advice`.

use serde_json::Value;

#[derive(Debug)]
pub enum LandGateAdviseError {
    Invalid(String),
    Unavailable(String),
}

#[async_trait::async_trait]
pub trait LandGateAdvising: Send + Sync {
    async fn advise(&self, request: Value) -> Result<Value, LandGateAdviseError>;
}
