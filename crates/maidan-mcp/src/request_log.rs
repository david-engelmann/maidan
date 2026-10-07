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

/// The client chooses the method, the tool name and the argument names, so
/// each is logged only when it is shaped like a name. Anything else could be
/// data a client put in the wrong place.
fn name_like(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'/' | b'-' | b'.'))
}

/// The argument names a request carried, sorted and comma-joined, and how
/// many were left out for not being shaped like a name.
fn arg_keys(args: &Value) -> (String, usize) {
    let Some(args) = args.as_object() else {
        return (String::new(), 0);
    };
    let mut named: Vec<&str> = args
        .keys()
        .map(String::as_str)
        .filter(|key| name_like(key))
        .collect();
    named.sort_unstable();
    let unnamed = args.len() - named.len();
    (named.join(","), unnamed)
}

/// What the log says about one request, kept apart from the emitting so a
/// test can check it without a subscriber.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub method: String,
    pub tool: String,
    pub arg_keys: String,
    pub unnamed_args: usize,
    pub outcome: &'static str,
    pub error_code: Option<i32>,
}

pub(crate) fn line(method: &str, params: &Value, result: &Result<Value, McpError>) -> Line {
    let (tool, args) = if method == "tools/call" {
        let tool = params
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| crate::tools::required_capability(name).is_ok())
            .unwrap_or("(unknown)");
        (tool, params.get("arguments").unwrap_or(&Value::Null))
    } else {
        ("-", params)
    };
    let (arg_keys, unnamed_args) = arg_keys(args);
    let (outcome, error_code) = match result {
        Ok(value) if value.get("isError") == Some(&Value::Bool(true)) => ("tool_error", None),
        Ok(_) => ("ok", None),
        Err(err) => ("error", Some(err.to_jsonrpc().code)),
    };
    Line {
        method: if name_like(method) {
            method.to_string()
        } else {
            "(other)".to_string()
        },
        tool: tool.to_string(),
        arg_keys,
        unnamed_args,
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
        method = %line.method,
        tool = %line.tool,
        arg_keys = %line.arg_keys,
        unnamed_args = line.unnamed_args,
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

/// A key whose value a capture replaces, wherever it sits in the frame.
#[cfg(any(test, feature = "frame-capture"))]
fn sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "secret",
        "token",
        "password",
        "authorization",
        "api_key",
        "cookie",
    ]
    .iter()
    .any(|word| key.contains(word))
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

#[cfg(feature = "frame-capture")]
pub(crate) fn capture(
    request: &crate::protocol::JsonRpcRequest,
    response: &crate::protocol::JsonRpcResponse,
) {
    if !tracing::enabled!(target: FRAME_TARGET, tracing::Level::DEBUG) {
        return;
    }
    let frame = |value: Result<Value, serde_json::Error>| {
        value.map_or_else(
            |err| format!("(unserializable: {err})"),
            |v| redact(v).to_string(),
        )
    };
    tracing::debug!(
        target: FRAME_TARGET,
        request = %frame(serde_json::to_value(request)),
        response = %frame(serde_json::to_value(response)),
        "mcp frame"
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_tool_call_logs_its_tool_and_argument_names_and_no_value() {
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
    fn a_name_the_client_made_up_is_not_logged() {
        let params = json!({
            "name": "Bearer sk-live-0123456789",
            "arguments": { "ok_key": 1, "Bearer sk-live-0123456789": 2 }
        });
        let logged = line(
            "tools/call",
            &params,
            &Err(McpError::MethodNotFound("tools/x".into())),
        );
        assert_eq!(logged.tool, "(unknown)");
        assert_eq!(logged.arg_keys, "ok_key");
        assert_eq!(logged.unnamed_args, 1);
        assert_eq!(logged.outcome, "error");
        assert!(logged.error_code.is_some());
        assert!(!format!("{logged:?}").contains("sk-live"));

        let odd = line("tools/list\nforged", &json!({}), &Ok(json!({})));
        assert_eq!(odd.method, "(other)");
    }

    #[test]
    fn a_tool_that_reports_an_error_is_a_tool_error() {
        let params = json!({ "name": "whoami", "arguments": {} });
        let logged = line("tools/call", &params, &Ok(json!({ "isError": true })));
        assert_eq!(logged.outcome, "tool_error");
    }

    #[test]
    fn a_captured_frame_hides_every_secret_shaped_key_at_any_depth() {
        let frame = redact(json!({
            "params": {
                "arguments": { "body": "hello", "api_key": "k", "nested": [{ "Token": "t" }] }
            },
            "result": { "secret": "s", "fencing_token": 7, "client_secret_hash": "h" }
        }));
        let text = frame.to_string();
        assert!(text.contains("hello"));
        for leaked in ["\"k\"", "\"t\"", "\"s\"", "7", "\"h\""] {
            assert!(!text.contains(leaked), "{leaked} leaked: {text}");
        }
    }
}
