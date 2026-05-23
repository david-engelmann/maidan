//! MCP dispatcher. Takes JSON-RPC requests and returns responses.
//! Transport-agnostic; `maidan-server` wraps it behind `POST /mcp`.

use std::sync::Arc;

use maidan_search::Search;
use maidan_store::Store;
use serde_json::{json, Value};

use crate::error::McpError;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::{resources, tools};

const MCP_VERSION: &str = "2024-11-05";

#[derive(Clone)]
pub struct McpServer {
    store: Arc<dyn Store>,
    search: Arc<dyn Search>,
    server_name: String,
    server_version: String,
}

impl McpServer {
    pub fn new(store: Arc<dyn Store>, search: Arc<dyn Search>) -> Self {
        Self {
            store,
            search,
            server_name: "maidan".into(),
            server_version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub async fn handle(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        match self.dispatch(&request).await {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(err) => {
                tracing::debug!(method = %request.method, error = %err, "mcp dispatch error");
                JsonRpcResponse::failure(id, err.to_jsonrpc())
            }
        }
    }

    async fn dispatch(&self, request: &JsonRpcRequest) -> Result<Value, McpError> {
        match request.method.as_str() {
            "initialize" => self.initialize().await,
            "tools/list" => Ok(json!({ "tools": tools::catalog() })),
            "tools/call" => self.tools_call(&request.params).await,
            "resources/list" => Ok(json!({ "resources": resources::catalog() })),
            "resources/read" => self.resources_read(&request.params).await,
            other => Err(McpError::MethodNotFound(other.into())),
        }
    }

    async fn initialize(&self) -> Result<Value, McpError> {
        Ok(json!({
            "protocolVersion": MCP_VERSION,
            "capabilities": {
                "tools": {},
                "resources": {}
            },
            "serverInfo": {
                "name": self.server_name,
                "version": self.server_version
            }
        }))
    }

    async fn tools_call(&self, params: &Value) -> Result<Value, McpError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing tool name".into()))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        tools::dispatch(&self.store, &self.search, name, &args).await
    }

    async fn resources_read(&self, params: &Value) -> Result<Value, McpError> {
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        resources::read(&self.store, uri).await
    }
}
