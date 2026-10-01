//! JSON-RPC 2.0 envelope types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
        Self::rejected(JsonRpcError::parse_error())
    }

    /// The answer to a body that is not a request: its id could not be read,
    /// so it is null.
    pub fn rejected(error: JsonRpcError) -> Self {
        Self::failure(serde_json::Value::Null, error)
    }
}

impl JsonRpcError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "parse error".into(),
            data: None,
        }
    }

    /// JSON that is not a request object: JSON-RPC 2.0's Invalid Request.
    pub fn invalid_request(why: &str) -> Self {
        Self {
            code: -32600,
            message: format!("invalid request: {why}"),
            data: None,
        }
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

/// What a JSON-RPC body decodes to: one request, or a batch whose items are
/// decoded one at a time so a bad item is answered without failing the rest.
#[derive(Debug)]
pub enum RequestBody {
    Single(JsonRpcRequest),
    Batch(Vec<Result<JsonRpcRequest, JsonRpcError>>),
}

/// Decode a `POST /mcp` body: a request object or a batch of them. The error is
/// answered with [`JsonRpcResponse::rejected`]. A public function rather than a step inside the
/// handler so the parser can be fuzzed (`fuzz/fuzz_targets/mcp_request.rs`).
pub fn parse_body(body: &[u8]) -> Result<RequestBody, JsonRpcError> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| JsonRpcError::parse_error())?;
    match value {
        serde_json::Value::Array(items) if items.is_empty() => {
            Err(JsonRpcError::invalid_request("empty batch"))
        }
        serde_json::Value::Array(items) => Ok(RequestBody::Batch(
            items.into_iter().map(request_from_value).collect(),
        )),
        other => request_from_value(other).map(RequestBody::Single),
    }
}

/// Decode a body or stdio line that carries exactly one request, as the
/// streamable-HTTP and stdio transports do. It reads the body the way
/// [`parse_body`] does, so every transport agrees on what a body says.
pub fn parse_request(body: &[u8]) -> Result<JsonRpcRequest, JsonRpcError> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| JsonRpcError::parse_error())?;
    request_from_value(value)
}

/// A JSON number with no fractional part. `Number::is_i64` and `is_u64` are
/// false once the value does not fit, even when it is a whole number: those
/// are kept as `f64`, and every finite `f64` at magnitude 2^53 or above is a
/// whole number. A fractional value such as `1.5` is not an id.
fn number_is_integer(number: &serde_json::Number) -> bool {
    if number.is_i64() || number.is_u64() {
        return true;
    }
    #[allow(clippy::float_cmp)]
    number
        .as_f64()
        .is_some_and(|value| value.is_finite() && value.fract() == 0.0)
}

/// Through a JSON value rather than straight into the struct: serde reads a
/// struct from an array by position and skips unknown members without
/// checking their UTF-8, so a direct read accepted `["2.0",1,"tools/call"]`
/// and bodies that are not JSON at all, which a gateway reading the body as
/// JSON would route differently or refuse.
///
/// JSON that does not make a request is Invalid Request (`-32600`), not a
/// parse error: that code is for bytes that are not JSON. MCP forbids a null
/// id, which JSON-RPC allows, and read as an absent one it turned a request
/// into a notification that was run and never answered.
fn request_from_value(value: serde_json::Value) -> Result<JsonRpcRequest, JsonRpcError> {
    let serde_json::Value::Object(object) = &value else {
        return Err(JsonRpcError::invalid_request("a request is a JSON object"));
    };
    if object.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0") {
        return Err(JsonRpcError::invalid_request("jsonrpc must be \"2.0\""));
    }
    match object.get("id") {
        None | Some(serde_json::Value::String(_)) => {}
        // i64/u64 miss a whole number that does not fit: serde_json stores it
        // as f64. It is still an integer, and the reply must echo it. A
        // fraction (1.5) is not an integer and stays invalid.
        Some(serde_json::Value::Number(n)) if number_is_integer(n) => {}
        Some(_) => {
            return Err(JsonRpcError::invalid_request(
                "id must be a string or an integer",
            ))
        }
    }
    if !object
        .get("method")
        .is_some_and(serde_json::Value::is_string)
    {
        return Err(JsonRpcError::invalid_request("method must be a string"));
    }
    serde_json::from_value(value).map_err(|_| JsonRpcError::invalid_request("malformed request"))
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

        // The struct reads a JSON `null` id as absent, because `id` is
        // `Option<Value>`; `parse_request` refuses a null id before this.
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

    #[test]
    fn a_request_is_an_object_on_every_transport() {
        let positional = br#"["2.0",1,"tools/call",{"name":"get_inbox"}]"#;
        assert!(parse_request(positional).is_err());
        let Ok(RequestBody::Batch(items)) = parse_body(br#"[["2.0",1,"tools/list"]]"#) else {
            panic!("a batch is a batch");
        };
        assert!(items[0].is_err(), "a positional batch item was read");
    }

    #[test]
    fn json_that_is_not_a_request_is_an_invalid_request_and_bytes_that_are_not_json_a_parse_error()
    {
        fn code(result: Result<JsonRpcRequest, JsonRpcError>) -> i32 {
            result.expect_err("refused").code
        }
        for body in [
            &b"{not json"[..],
            b"",
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"x\":\"\xcd\"}",
        ] {
            assert_eq!(code(parse_request(body)), -32700, "{body:?}");
            assert_eq!(parse_body(body).expect_err("refused").code, -32700);
        }
        for body in [
            &br#"1"#[..],
            br#""ping""#,
            br#"null"#,
            br#"["2.0",1,"tools/list"]"#,
            br#"{"jsonrpc":"2.0","id":1}"#,
            br#"{"jsonrpc":"2.0","id":1,"method":7}"#,
            br#"{"id":1,"method":"ping"}"#,
            br#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
            br#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
            br#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#,
            br#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
        ] {
            assert_eq!(
                code(parse_request(body)),
                -32600,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        let Ok(RequestBody::Batch(items)) = parse_body(
            br#"[1,{"jsonrpc":"2.0","method":"ping"},{"jsonrpc":"2.0","id":2,"method":"ping"}]"#,
        ) else {
            panic!("a batch is a batch");
        };
        assert_eq!(items[0].as_ref().expect_err("not a request").code, -32600);
        assert!(
            items[1].as_ref().is_ok_and(|r| r.id.is_none()),
            "a notification"
        );
        assert!(items[2].as_ref().is_ok_and(|r| r.id == Some(json!(2))));
        assert_eq!(parse_body(b"[]").expect_err("empty batch").code, -32600);
        assert_eq!(parse_body(b"7").expect_err("not a request").code, -32600);
    }

    #[test]
    fn a_body_that_is_not_json_is_refused_even_where_serde_would_skip_it() {
        // Found by the `mcp_request` fuzz target: invalid UTF-8 inside an
        // unknown member was skipped unchecked by a direct struct read.
        let body = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"x\":\"\xcd\"}";
        assert!(parse_request(body).is_err());
        assert!(parse_body(body).is_err());
    }

    #[test]
    fn single_and_batch_transports_read_a_request_alike() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","method":"tools/call"}"#;
        let single = parse_request(body).expect("parses");
        let Ok(RequestBody::Single(same)) = parse_body(body) else {
            panic!("a single request");
        };
        assert_eq!(single, same);
    }

    #[test]
    fn a_number_id_is_read_as_the_number_the_client_sent() {
        // Found by the `mcp_request` fuzz target: serde_json's default float
        // parse is best effort, so `1.5555555555555555e92` read as
        // 1.5555555555555558e92 and the reply echoed an id the client never
        // sent. The long integer is the fuzz input; it prints as that float.
        for literal in [
            "1.5555555555555555e92".to_string(),
            format!("1{}", "5".repeat(92)),
        ] {
            let body = format!(r#"{{"jsonrpc":"2.0","id":{literal},"method":"ping"}}"#);
            let request = parse_request(body.as_bytes()).expect("parses");
            let sent: f64 = literal.parse().expect("a number");
            assert_eq!(
                request.id.as_ref().and_then(serde_json::Value::as_f64),
                Some(sent),
                "{literal}"
            );
            let printed = serde_json::to_vec(&request).expect("serializes");
            assert_eq!(parse_request(&printed).expect("reparses"), request);
        }
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
