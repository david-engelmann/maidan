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

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden: {0}")]
    Forbidden(String),
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
            Self::Unauthorized => JsonRpcError {
                code: -32001,
                message: "unauthorized".into(),
                data: None,
            },
            Self::Forbidden(msg) => JsonRpcError {
                code: -32003,
                message: format!("forbidden: {msg}"),
                data: None,
            },
        }
    }
}

impl From<maidan_auth::AuthError> for McpError {
    fn from(err: maidan_auth::AuthError) -> Self {
        use maidan_auth::AuthError;
        match err {
            AuthError::Unauthorized => Self::Unauthorized,
            AuthError::Forbidden(msg) => Self::Forbidden(msg),
            AuthError::Store(e) => Self::Internal(e.to_string()),
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

impl From<maidan_search::SearchError> for McpError {
    fn from(err: maidan_search::SearchError) -> Self {
        use maidan_search::SearchError;
        match err {
            SearchError::InvalidQuery(m) => Self::InvalidParams(m),
            SearchError::Unsupported(f) => {
                Self::InvalidParams(format!("not supported by backend: {f}"))
            }
            SearchError::Database(e) => Self::Internal(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variant_maps_to_its_jsonrpc_code() {
        let cases = [
            (McpError::MethodNotFound("m".into()), -32601),
            (McpError::InvalidParams("p".into()), -32602),
            (McpError::Internal("i".into()), -32603),
            (McpError::NotFound, -32004),
            (McpError::Unauthorized, -32001),
            (McpError::Forbidden("f".into()), -32003),
        ];
        for (err, expected_code) in cases {
            assert_eq!(err.to_jsonrpc().code, expected_code, "{err:?}");
        }
    }

    #[test]
    fn parameterized_variants_carry_their_message() {
        assert!(McpError::MethodNotFound("tools/x".into())
            .to_jsonrpc()
            .message
            .contains("tools/x"));
        assert!(McpError::Forbidden("no write".into())
            .to_jsonrpc()
            .message
            .contains("no write"));
        // Unit variants use a fixed message and never expose internals.
        assert_eq!(McpError::Unauthorized.to_jsonrpc().message, "unauthorized");
    }

    #[test]
    fn auth_errors_map_to_unauthorized_forbidden_and_internal() {
        assert!(matches!(
            McpError::from(maidan_auth::AuthError::Unauthorized),
            McpError::Unauthorized
        ));
        assert!(matches!(
            McpError::from(maidan_auth::AuthError::Forbidden("x".into())),
            McpError::Forbidden(_)
        ));
        assert!(matches!(
            McpError::from(maidan_auth::AuthError::Store(
                maidan_store::StoreError::NotFound
            )),
            McpError::Internal(_)
        ));
    }

    #[test]
    fn store_not_found_and_invalid_input_map_distinctly() {
        assert!(matches!(
            McpError::from(maidan_store::StoreError::NotFound),
            McpError::NotFound
        ));
        assert!(matches!(
            McpError::from(maidan_store::StoreError::InvalidInput("bad".into())),
            McpError::InvalidParams(_)
        ));
        assert!(matches!(
            McpError::from(maidan_store::StoreError::Conflict("dup".into())),
            McpError::Internal(_)
        ));
    }

    #[test]
    fn serde_error_becomes_invalid_params() {
        let serde_err = serde_json::from_str::<serde_json::Value>("{ not json").unwrap_err();
        assert!(matches!(
            McpError::from(serde_err),
            McpError::InvalidParams(_)
        ));
    }
}
