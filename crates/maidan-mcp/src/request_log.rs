//! The MCP request log: one `info` event per request on the target
//! `maidan_mcp::request`, on at the default filter. It carries the method, the
//! tool, the names of the arguments, the principal, the transport, the latency
//! and the outcome. It never carries an argument value or a result, because
//! those hold message text, the secrets a tool stores and the tokens a tool
//! mints.
//!
//! The one exception is the full-frame capture behind the `frame-capture`
//! feature, for a developer reproducing a client's exchange. A release build
//! refuses the feature, so no setting can turn capture on in production, and a
//! dev build emits frames only when `MAIDAN_LOG` enables `maidan_mcp::frame`.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Duration;

use maidan_auth::AuthContext;
use serde_json::Value;

use crate::{error::McpError, profiles::Profile, subscriptions::McpSession};

#[cfg(all(feature = "frame-capture", not(debug_assertions)))]
compile_error!(
    "frame-capture logs whole MCP frames and is for dev builds only; a release build must not enable it"
);

pub const TARGET: &str = "maidan_mcp::request";

#[cfg(feature = "frame-capture")]
pub const FRAME_TARGET: &str = "maidan_mcp::frame";

/// How a request reached the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transport {
    Http,
    Streamable,
    Stdio,
    Slash,
}

impl Transport {
    pub(crate) fn of(session: &McpSession) -> Self {
        match session {
            McpSession::Stateless => Self::Http,
            McpSession::Streamable(_) => Self::Streamable,
            McpSession::Stdio => Self::Stdio,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Streamable => "streamable",
            Self::Stdio => "stdio",
            Self::Slash => "slash",
        }
    }
}

/// The methods the server answers. The client chooses the method string, so
/// any other is logged as `(other)`: a credential pasted into it must not
/// reach the log, whatever it looks like.
const METHODS: &[&str] = &[
    "initialize",
    "notifications/cancelled",
    "notifications/initialized",
    "ping",
    "prompts/get",
    "prompts/list",
    "resources/list",
    "resources/read",
    "resources/subscribe",
    "resources/templates/list",
    "resources/unsubscribe",
    "server/discover",
    "tools/call",
    "tools/list",
];

/// The protocol's own parameter names, logged for methods other than
/// `tools/call`.
const PROTOCOL_PARAMS: &[&str] = &[
    "_meta",
    "arguments",
    "capabilities",
    "clientInfo",
    "cursor",
    "name",
    "protocolVersion",
    "uri",
];

/// Each tool's declared argument names, read from the catalog once. A tool
/// that is not in it is unknown, and only a declared name is ever logged.
fn declared_args(tool: &str) -> Option<&'static HashSet<String>> {
    static DECLARED: LazyLock<HashMap<String, HashSet<String>>> = LazyLock::new(|| {
        crate::tools::catalog()
            .into_iter()
            .filter_map(|tool| {
                let name = tool["name"].as_str()?.to_string();
                let args = tool["inputSchema"]["properties"]
                    .as_object()
                    .map(|properties| properties.keys().cloned().collect())
                    .unwrap_or_default();
                Some((name, args))
            })
            .collect()
    });
    DECLARED.get(tool)
}

/// The argument names a request carried that `known` lists, sorted and
/// comma-joined, and how many it carried that `known` does not list.
fn arg_keys(args: &Value, known: impl Fn(&str) -> bool) -> (String, usize) {
    let Some(args) = args.as_object() else {
        return (String::new(), 0);
    };
    let mut declared: Vec<&str> = args
        .keys()
        .map(String::as_str)
        .filter(|key| known(key))
        .collect();
    declared.sort_unstable();
    let undeclared = args.len() - declared.len();
    (declared.join(","), undeclared)
}

/// What the log says about one request, kept apart from the emitting so a
/// test can check it without a subscriber.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub method: &'static str,
    pub tool: String,
    pub arg_keys: String,
    pub undeclared_args: usize,
    pub outcome: &'static str,
    pub error_code: Option<i32>,
}

pub(crate) fn line(method: &str, params: &Value, result: &Result<Value, McpError>) -> Line {
    let method = METHODS
        .iter()
        .copied()
        .find(|known| *known == method)
        .unwrap_or("(other)");
    let (tool, (arg_keys, undeclared_args)) = if method == "tools/call" {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let args = params.get("arguments").unwrap_or(&Value::Null);
        match declared_args(name) {
            Some(declared) => (name, arg_keys(args, |key| declared.contains(key))),
            None => ("(unknown)", arg_keys(args, |_| false)),
        }
    } else {
        ("-", arg_keys(params, |key| PROTOCOL_PARAMS.contains(&key)))
    };
    let (outcome, error_code) = match result {
        Ok(value) if value.get("isError") == Some(&Value::Bool(true)) => ("tool_error", None),
        Ok(_) => ("ok", None),
        Err(err) => ("error", Some(err.to_jsonrpc().code)),
    };
    Line {
        method,
        tool: tool.to_string(),
        arg_keys,
        undeclared_args,
        outcome,
        error_code,
    }
}

pub(crate) fn record(
    method: &str,
    params: &Value,
    auth: &AuthContext,
    transport: Transport,
    profile: Option<Profile>,
    latency: Duration,
    result: &Result<Value, McpError>,
) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let line = line(method, params, result);
    let optional = |id: Option<String>| id.unwrap_or_else(|| "-".to_string());
    tracing::info!(
        target: TARGET,
        method = line.method,
        tool = %line.tool,
        arg_keys = %line.arg_keys,
        undeclared_args = line.undeclared_args,
        transport = transport.as_str(),
        profile = profile.map_or("-", Profile::name),
        workspace_id = %auth.workspace_id,
        member_id = %auth.member_id,
        actor_id = %auth.actor_id,
        app_installation_id = %optional(auth.app_installation_id.map(|id| id.to_string())),
        delegation_grant_id = %optional(auth.delegation_grant_id.map(|id| id.to_string())),
        bypass = auth.bypass,
        latency_ms = u64::try_from(latency.as_millis()).unwrap_or(u64::MAX),
        outcome = line.outcome,
        error_code = line.error_code,
        "mcp request"
    );
}

/// A key whose value a capture replaces, wherever it sits in the frame. The
/// key is compared with its case and separators dropped, so `apiKey`,
/// `api_key` and `API-KEY` are one name.
#[cfg(any(test, feature = "frame-capture"))]
fn sensitive(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    [
        "apikey",
        "authorization",
        "bearer",
        "cookie",
        "credential",
        "passwd",
        "password",
        "privatekey",
        "secret",
        "token",
    ]
    .iter()
    .any(|word| key.contains(word))
}

/// A tool whose arguments or result are a credential with no telling key: a
/// secret's value, a minted or rotated token, a share ticket, a grant.
#[cfg(any(test, feature = "frame-capture"))]
fn carries_credentials(tool: &str) -> bool {
    ["secret", "token", "ticket", "grant"]
        .iter()
        .any(|word| tool.contains(word))
}

#[cfg(any(test, feature = "frame-capture"))]
fn redact(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let value = if sensitive(&key) {
                        Value::String("[redacted]".into())
                    } else {
                        redact(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(redact).collect()),
        other => other,
    }
}

/// A tool result's text is usually JSON, which key redaction cannot see
/// into while it is a string, so each text item that parses is redacted too.
#[cfg(any(test, feature = "frame-capture"))]
fn redact_text_content(mut response: Value) -> Value {
    if let Some(Value::Array(items)) = response.pointer_mut("/result/content") {
        for item in items {
            if let Some(Value::String(text)) = item.get_mut("text") {
                if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                    *text = redact(parsed).to_string();
                }
            }
        }
    }
    response
}

/// The request and response as a capture logs them: a credential-carrying
/// tool's arguments and result withheld outright, everything else with
/// secret-shaped keys redacted at any depth, inside result text included.
#[cfg(any(test, feature = "frame-capture"))]
fn captured_frame(
    request: &crate::protocol::JsonRpcRequest,
    response: &crate::protocol::JsonRpcResponse,
) -> (Value, Value) {
    let tool = if request.method == "tools/call" {
        request
            .params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
    } else {
        ""
    };
    let mut request = serde_json::to_value(request).unwrap_or(Value::Null);
    let mut response = serde_json::to_value(response).unwrap_or(Value::Null);
    if carries_credentials(tool) {
        let withheld = Value::String(format!("[withheld: {tool} carries credentials]"));
        if let Some(args) = request.pointer_mut("/params/arguments") {
            *args = withheld.clone();
        }
        if let Some(result) = response.pointer_mut("/result") {
            *result = withheld;
        }
    }
    (redact(request), redact(redact_text_content(response)))
}

#[cfg(feature = "frame-capture")]
pub(crate) fn capture(
    request: &crate::protocol::JsonRpcRequest,
    response: &crate::protocol::JsonRpcResponse,
) {
    if !tracing::enabled!(target: FRAME_TARGET, tracing::Level::DEBUG) {
        return;
    }
    let (request, response) = captured_frame(request, response);
    tracing::debug!(
        target: FRAME_TARGET,
        request = %request,
        response = %response,
        "mcp frame"
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

    #[test]
    fn a_tool_call_logs_its_tool_and_declared_argument_names_and_no_value() {
        let params = json!({
            "name": "post_message",
            "arguments": { "thread_id": "t-1", "body": "the launch code is 0000" }
        });
        let logged = line("tools/call", &params, &Ok(json!({ "content": [] })));
        assert_eq!(logged.tool, "post_message");
        assert_eq!(logged.arg_keys, "body,thread_id");
        assert_eq!(logged.outcome, "ok");
        assert!(!format!("{logged:?}").contains("launch code"));
    }

    #[test]
    fn a_name_the_client_chose_is_logged_only_when_the_server_declares_it() {
        let credential = "sk-live-0123456789";
        let known_tool = json!({
            "name": "post_message",
            "arguments": { "body": "x", credential: 1 }
        });
        let logged = line("tools/call", &known_tool, &Ok(json!({})));
        assert_eq!(logged.arg_keys, "body");
        assert_eq!(logged.undeclared_args, 1);

        let unknown_tool = json!({ "name": credential, "arguments": { "body": 1 } });
        let logged = line(
            "tools/call",
            &unknown_tool,
            &Err(McpError::MethodNotFound("tools/x".into())),
        );
        assert_eq!(logged.tool, "(unknown)");
        assert_eq!(logged.arg_keys, "");
        assert_eq!(logged.undeclared_args, 1);
        assert_eq!(logged.outcome, "error");
        assert!(logged.error_code.is_some());

        let odd_method = line(
            credential,
            &json!({ credential: 1, "uri": "u" }),
            &Ok(json!({})),
        );
        assert_eq!(odd_method.method, "(other)");
        assert_eq!(odd_method.arg_keys, "uri");
        for logged in [
            format!("{:?}", line("tools/call", &known_tool, &Ok(json!({})))),
            format!("{odd_method:?}"),
        ] {
            assert!(!logged.contains("sk-live"), "{logged}");
        }
    }

    #[test]
    fn every_method_the_server_dispatches_is_logged_by_name() {
        for method in METHODS {
            assert_eq!(line(method, &json!({}), &Ok(json!({}))).method, *method);
        }
    }

    #[test]
    fn a_tool_that_reports_an_error_is_a_tool_error() {
        let params = json!({ "name": "whoami", "arguments": {} });
        let logged = line("tools/call", &params, &Ok(json!({ "isError": true })));
        assert_eq!(logged.outcome, "tool_error");
    }

    fn call(tool: &str, arguments: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: json!({ "name": tool, "arguments": arguments }),
        }
    }

    #[test]
    fn a_captured_frame_hides_every_secret_shaped_key_at_any_depth() {
        let request = call(
            "post_message",
            json!({ "body": "hello", "apiKey": "k1", "nested": [{ "API-KEY": "k2", "Token": "t" }] }),
        );
        let result = json!({
            "content": [{ "type": "text", "text": "{\"secret\":\"s\",\"fencing_token\":7,\"note\":\"kept\"}" }],
            "client_secret_hash": "h"
        });
        let (request, response) =
            captured_frame(&request, &JsonRpcResponse::success(json!(1), result));
        let text = format!("{request} {response}");
        assert!(text.contains("hello") && text.contains("kept"), "{text}");
        for leaked in ["k1", "k2", "\"t\"", "\\\"s\\\"", "7", "\"h\""] {
            assert!(!text.contains(leaked), "{leaked} leaked: {text}");
        }
    }

    #[test]
    fn a_credential_tools_arguments_and_result_are_withheld_from_a_capture() {
        let request = call("create_secret", json!({ "name": "db", "value": "hunter2" }));
        let result = json!({ "content": [{ "type": "text", "text": "value: hunter3" }] });
        let (request, response) =
            captured_frame(&request, &JsonRpcResponse::success(json!(1), result));
        let text = format!("{request} {response}");
        assert!(text.contains("withheld"), "{text}");
        assert!(!text.contains("hunter"), "{text}");
        for tool in [
            "resolve_secret",
            "rotate_token",
            "delegate_token",
            "create_share_ticket",
            "create_delegation_grant",
        ] {
            assert!(carries_credentials(tool), "{tool}");
        }
    }
}
