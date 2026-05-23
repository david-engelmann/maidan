//! MCP dispatcher. Takes JSON-RPC requests and returns responses.
//! Transport-agnostic; `maidan-server` wraps it behind `POST /mcp`.

use std::sync::Arc;

use maidan_artifacts::ArtifactStore;
use maidan_auth::AuthContext;
use maidan_search::Search;
use maidan_store::Store;
use serde_json::{json, Value};

use crate::error::McpError;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::{prompts, resources, tools};

const MCP_VERSION: &str = "2024-11-05";

#[derive(Clone)]
pub struct McpServer {
    store: Arc<dyn Store>,
    artifacts: Arc<dyn ArtifactStore>,
    search: Arc<dyn Search>,
    server_name: String,
    server_version: String,
}

impl McpServer {
    pub fn new(
        store: Arc<dyn Store>,
        artifacts: Arc<dyn ArtifactStore>,
        search: Arc<dyn Search>,
    ) -> Self {
        Self {
            store,
            artifacts,
            search,
            server_name: "maidan".into(),
            server_version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub async fn handle(&self, request: JsonRpcRequest, auth: &AuthContext) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        match self.dispatch(&request, auth).await {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(err) => {
                tracing::debug!(method = %request.method, error = %err, "mcp dispatch error");
                JsonRpcResponse::failure(id, err.to_jsonrpc())
            }
        }
    }

    async fn dispatch(
        &self,
        request: &JsonRpcRequest,
        auth: &AuthContext,
    ) -> Result<Value, McpError> {
        match request.method.as_str() {
            "initialize" => self.initialize().await,
            "tools/list" => Ok(json!({ "tools": tools::catalog() })),
            "tools/call" => self.tools_call(&request.params, auth).await,
            "resources/list" => Ok(json!({ "resources": resources::catalog() })),
            "resources/read" => self.resources_read(&request.params, auth).await,
            "prompts/list" => Ok(json!({ "prompts": prompts::catalog() })),
            "prompts/get" => self.prompts_get(&request.params, auth).await,
            other => Err(McpError::MethodNotFound(other.into())),
        }
    }

    async fn initialize(&self) -> Result<Value, McpError> {
        Ok(json!({
            "protocolVersion": MCP_VERSION,
            "capabilities": {
                "tools": {},
                "resources": {},
                "prompts": {}
            },
            "serverInfo": {
                "name": self.server_name,
                "version": self.server_version
            }
        }))
    }

    async fn tools_call(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing tool name".into()))?;
        if !auth.bypass {
            let cap = tools::required_capability(name)?;
            auth.require_capability(cap).map_err(McpError::from)?;
        }
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        tools::dispatch(
            &self.store,
            &self.artifacts,
            &self.search,
            auth,
            name,
            &args,
        )
        .await
    }

    async fn prompts_get(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing prompt name".into()))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        prompts::get(&self.store, name, &args).await
    }

    async fn resources_read(&self, params: &Value, auth: &AuthContext) -> Result<Value, McpError> {
        if !auth.bypass {
            auth.require_capability(maidan_auth::capability::WORKSPACE_READ)
                .map_err(McpError::from)?;
        }
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("missing uri".into()))?;
        resources::read(&self.store, &self.artifacts, uri).await
    }
}
