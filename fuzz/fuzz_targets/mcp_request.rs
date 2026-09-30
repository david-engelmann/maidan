//! An MCP request body arrives from any client with a token. Three things must
//! hold for whatever it decodes to:
//!
//! - a request prints to JSON that decodes to the same request;
//! - the method dispatched is the method a gateway reading the body as plain
//!   JSON sees, so a gateway routing or authorizing on the body (SEP-2243)
//!   routes the call Maidan makes;
//! - the argument gates that run before a tool's handler agree with the typed
//!   decode the handler makes: a `workspace_id` or member tool's `member_id`
//!   that passes the gate but decodes to someone else is a gate bypass.
#![no_main]

use libfuzzer_sys::fuzz_target;
use maidan_auth::AuthContext;
use maidan_mcp::protocol::{parse_body, parse_request, RequestBody};
use maidan_mcp::{tools, JsonRpcRequest};
use maidan_types::{MemberId, WorkspaceId};
use serde_json::Value;

fn caller() -> AuthContext {
    AuthContext::from_session(
        MemberId(uuid::Uuid::from_u128(1)),
        WorkspaceId(uuid::Uuid::from_u128(2)),
        vec!["workspace:read".into()],
    )
}

fn round_trips(request: &JsonRpcRequest) {
    let printed = serde_json::to_vec(request).expect("a request serializes");
    let again = parse_request(&printed)
        .unwrap_or_else(|_| panic!("{request:?} printed as {printed:?}, which does not parse"));
    assert_eq!(&again, request, "{printed:?} is not stable");
}

fn gateway_sees(request: &JsonRpcRequest, raw: &Value) {
    assert_eq!(
        raw.get("method").and_then(Value::as_str),
        Some(request.method.as_str()),
        "dispatched {:?} from {raw}",
        request.method
    );
}

fn gates_agree_with_handlers(request: &JsonRpcRequest) {
    if request.method != "tools/call" {
        return;
    }
    let Ok((name, args)) = tools::tool_call(&request.params) else {
        return;
    };
    let auth = caller();
    if tools::check_argument_scope(&auth, name, &args).is_err() {
        return;
    }
    if let Some(Ok(ws)) = args
        .get("workspace_id")
        .map(|v| serde_json::from_value::<WorkspaceId>(v.clone()))
    {
        assert_eq!(ws, auth.workspace_id, "{name} passed naming {ws:?}");
    }
    let Some(Ok(member)) = args
        .get("member_id")
        .map(|v| serde_json::from_value::<MemberId>(v.clone()))
    else {
        return;
    };
    if member == auth.member_id {
        return;
    }
    // Only member tools gate `member_id`. Ask the gate whether this tool is
    // one, with an argument no decode can disagree about.
    let mut probe = serde_json::Map::new();
    probe.insert(
        "member_id".into(),
        Value::String(uuid::Uuid::from_u128(3).to_string()),
    );
    let member_tool = tools::check_argument_scope(&auth, name, &Value::Object(probe)).is_err();
    assert!(
        !member_tool,
        "{name} passed the member gate for {member:?} with {args}"
    );
}

fuzz_target!(|data: &[u8]| {
    if let Ok(single) = parse_request(data) {
        match parse_body(data) {
            Ok(RequestBody::Single(same)) => assert_eq!(same, single, "the transports disagree"),
            other => panic!("streamable read {single:?}, POST /mcp read {other:?}"),
        }
    }
    let Ok(body) = parse_body(data) else {
        return;
    };
    let raw: Value = serde_json::from_slice(data).expect("parse_body accepted it");
    match body {
        RequestBody::Single(request) => {
            round_trips(&request);
            gateway_sees(&request, &raw);
            gates_agree_with_handlers(&request);
        }
        RequestBody::Batch(items) => {
            let raw_items = raw.as_array().expect("a batch is an array");
            for (item, raw_item) in items.iter().zip(raw_items) {
                if let Ok(request) = item {
                    round_trips(request);
                    gateway_sees(request, raw_item);
                    gates_agree_with_handlers(request);
                }
            }
        }
    }
});
