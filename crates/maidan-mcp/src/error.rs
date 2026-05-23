//! MCP-internal errors. Mapped to JSON-RPC error responses by the
//! [`crate::McpServer`] dispatcher.

use thiserror::Error;

use crate::protocol::JsonRpcError;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("method not found: {0}")]
    MethodNotFound(String),

    #[error("invalid params: {0}")]
    InvalidParams(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("not found")]
    NotFound,
}

impl McpError {
    pub fn to_jsonrpc(&self) -> JsonRpcError {
        match self {
            Self::MethodNotFound(method) => JsonRpcError {
                code: -32601,
                message: format!("method not found: {method}"),
                data: None,
            },
            Self::InvalidParams(msg) => JsonRpcError {
                code: -32602,
                message: format!("invalid params: {msg}"),
                data: None,
            },
            Self::Internal(msg) => JsonRpcError {
                code: -32603,
                message: format!("internal error: {msg}"),
                data: None,
            },
            Self::NotFound => JsonRpcError {
                code: -32004,
                message: "resource not found".into(),
                data: None,
            },
        }
    }
}

impl From<maidan_store::StoreError> for McpError {
    fn from(err: maidan_store::StoreError) -> Self {
        match err {
            maidan_store::StoreError::NotFound => Self::NotFound,
            maidan_store::StoreError::InvalidInput(m) => Self::InvalidParams(m),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for McpError {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidParams(err.to_string())
    }
}
