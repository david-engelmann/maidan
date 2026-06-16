//! JSON-RPC 2.0 envelope types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: serde_json::Value, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
        }
    }

    /// Parse-failure response per the JSON-RPC spec (id = null when the
    /// request was unparseable).
    pub fn parse_error() -> Self {
        Self::failure(
            serde_json::Value::Null,
            JsonRpcError {
                code: -32700,
                message: "parse error".into(),
                data: None,
            },
        )
    }
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    #[test]
    fn request_deserializes_with_id_method_and_params() {
        let req: JsonRpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {"name": "search"}
        }))
        .expect("parse request");
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, Some(json!(7)));
        assert_eq!(req.method, "tools/call");
        assert_eq!(req.params, json!({"name": "search"}));
    }

    #[test]
    fn request_defaults_params_to_null_and_allows_string_or_null_id() {
        let no_params: JsonRpcRequest =
            serde_json::from_value(json!({"jsonrpc": "2.0", "id": "abc", "method": "ping"}))
                .expect("parse");
        assert_eq!(no_params.id, Some(json!("abc")));
        assert_eq!(no_params.params, serde_json::Value::Null);

        // An explicit JSON `null` id deserializes to `None` (like an absent id)
        // because `id` is `Option<Value>`.
        let null_id: JsonRpcRequest =
            serde_json::from_value(json!({"jsonrpc": "2.0", "id": null, "method": "x"}))
                .expect("parse null id");
        assert_eq!(null_id.id, None);
    }

    #[test]
    fn success_response_serializes_result_and_omits_error() {
        let resp = JsonRpcResponse::success(json!(1), json!({"ok": true}));
        let v = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], json!(1));
        assert_eq!(v["result"], json!({"ok": true}));
        assert!(v.get("error").is_none(), "error must be omitted on success");
    }

    #[test]
    fn failure_response_serializes_error_and_omits_result() {
        let resp = JsonRpcResponse::failure(
            json!(2),
            JsonRpcError {
                code: -32601,
                message: "method not found".into(),
                data: None,
            },
        );
        let v = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(v["error"]["code"], -32601);
        assert_eq!(v["error"]["message"], "method not found");
        assert!(v["error"].get("data").is_none(), "null data omitted");
        assert!(v.get("result").is_none(), "result omitted on failure");
    }

    #[test]
    fn parse_error_has_null_id_and_spec_code() {
        let v = serde_json::to_value(JsonRpcResponse::parse_error()).expect("serialize");
        assert_eq!(v["id"], serde_json::Value::Null);
        assert_eq!(v["error"]["code"], -32700);
    }

    #[test]
    fn notification_carries_method_and_params_without_id() {
        let n = JsonRpcNotification::new("notifications/message", json!({"level": "info"}));
        let v = serde_json::to_value(&n).expect("serialize");
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["method"], "notifications/message");
        assert_eq!(v["params"]["level"], "info");
        assert!(v.get("id").is_none(), "notifications have no id");
    }

    proptest! {
        /// Fuzz the error envelope: any (code, message, optional data) survives
        /// serialization with its fields intact and `data` omitted iff `None`.
        #[test]
        fn error_envelope_serializes_losslessly(
            code in any::<i32>(),
            message in ".*",
            has_data in any::<bool>(),
        ) {
            let data = has_data.then(|| json!({"detail": code}));
            let resp = JsonRpcResponse::failure(
                json!(1),
                JsonRpcError { code, message: message.clone(), data: data.clone() },
            );
            let v = serde_json::to_value(&resp).expect("serialize");
            prop_assert_eq!(v["error"]["code"].as_i64(), Some(code as i64));
            prop_assert_eq!(v["error"]["message"].as_str(), Some(message.as_str()));
            prop_assert_eq!(v["error"].get("data").is_some(), has_data);
        }

        /// Fuzz request parsing: arbitrary method/id round-trips through a JSON
        /// object into the typed request.
        #[test]
        fn request_parses_arbitrary_method_and_numeric_id(
            id in any::<i64>(),
            method in "[a-zA-Z/_]{1,32}",
        ) {
            let req: JsonRpcRequest = serde_json::from_value(json!({
                "jsonrpc": "2.0", "id": id, "method": method,
            }))
            .expect("parse");
            prop_assert_eq!(req.id, Some(json!(id)));
            prop_assert_eq!(req.method, method);
        }
    }
}
