//! Decoding a JSON-RPC 2.0 request (§9) into the operation it asks for. The
//! server's `POST /a2a/v1/rpc` handler runs these two steps and nothing else
//! before dispatch; they are public, pure functions so the decoder can be
//! fuzzed (`fuzz/fuzz_targets/a2a_request.rs`).

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::protocol::{
    A2aError, A2aErrorKind, CancelTaskRequest, DeleteTaskPushNotificationConfigRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, JsonRpcId,
    ListTaskPushNotificationConfigsRequest, ListTasksRequest, SendMessageRequest,
    SubscribeToTaskRequest, TaskPushNotificationConfig, JSONRPC_VERSION, METHOD_CANCEL_TASK,
    METHOD_CREATE_PUSH_NOTIFICATION_CONFIG, METHOD_DELETE_PUSH_NOTIFICATION_CONFIG,
    METHOD_GET_EXTENDED_AGENT_CARD, METHOD_GET_PUSH_NOTIFICATION_CONFIG, METHOD_GET_TASK,
    METHOD_LIST_PUSH_NOTIFICATION_CONFIGS, METHOD_LIST_TASKS, METHOD_SEND_MESSAGE,
    METHOD_SEND_STREAMING_MESSAGE, METHOD_SUBSCRIBE_TO_TASK,
};

/// A request's envelope, read by hand so an envelope error still answers
/// with the request's id where it has a usable one.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub id: JsonRpcId,
    pub method: String,
    /// An object, or `Null` when the request sent none.
    pub params: Value,
}

/// Read a request body's envelope. The error carries the id to answer with:
/// the request's own when it has a usable one, `Null` otherwise.
pub fn parse_envelope(body: Value) -> Result<Envelope, (JsonRpcId, A2aError)> {
    let invalid = |id: JsonRpcId, why: &str| (id, A2aError::new(A2aErrorKind::InvalidRequest, why));
    let Value::Object(mut body) = body else {
        return Err(invalid(JsonRpcId::Null, "a request must be a JSON object"));
    };
    let id = match body.remove("id") {
        Some(Value::String(s)) => JsonRpcId::Str(s),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(n) => JsonRpcId::Number(n),
            None => {
                return Err(invalid(
                    JsonRpcId::Null,
                    "id must be an integer or a string",
                ))
            }
        },
        // A2A has no notifications, and a null id is the one error responses
        // use, so a caller could not tell its answer from a rejection.
        None => return Err(invalid(JsonRpcId::Null, "id is required")),
        Some(_) => {
            return Err(invalid(
                JsonRpcId::Null,
                "id must be an integer or a string",
            ))
        }
    };
    if body.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        return Err(invalid(id, "jsonrpc must be \"2.0\""));
    }
    let method = match body.remove("method") {
        Some(Value::String(method)) => method,
        _ => return Err(invalid(id, "method must be a string")),
    };
    let params = body.remove("params").unwrap_or(Value::Null);
    if !matches!(params, Value::Object(_) | Value::Null) {
        return Err(invalid(id, "params must be an object"));
    }
    Ok(Envelope { id, method, params })
}

/// An A2A operation with its decoded parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    SendMessage(SendMessageRequest),
    SendStreamingMessage(SendMessageRequest),
    GetTask(GetTaskRequest),
    ListTasks(ListTasksRequest),
    CancelTask(CancelTaskRequest),
    SubscribeToTask(SubscribeToTaskRequest),
    CreatePushNotificationConfig(TaskPushNotificationConfig),
    GetPushNotificationConfig(GetTaskPushNotificationConfigRequest),
    ListPushNotificationConfigs(ListTaskPushNotificationConfigsRequest),
    DeletePushNotificationConfig(DeleteTaskPushNotificationConfigRequest),
    GetExtendedAgentCard,
}

/// Decode `params` as the request type `method` takes. An unknown method is
/// MethodNotFound, parameters of the wrong shape InvalidParams.
pub fn parse_operation(method: &str, params: Value) -> Result<Operation, A2aError> {
    Ok(match method {
        METHOD_SEND_MESSAGE => Operation::SendMessage(decode(method, params)?),
        METHOD_SEND_STREAMING_MESSAGE => Operation::SendStreamingMessage(decode(method, params)?),
        METHOD_GET_TASK => Operation::GetTask(decode(method, params)?),
        // Every ListTasks parameter is optional, so no params is no filter.
        METHOD_LIST_TASKS if params.is_null() => Operation::ListTasks(ListTasksRequest::default()),
        METHOD_LIST_TASKS => Operation::ListTasks(decode(method, params)?),
        METHOD_CANCEL_TASK => Operation::CancelTask(decode(method, params)?),
        METHOD_SUBSCRIBE_TO_TASK => Operation::SubscribeToTask(decode(method, params)?),
        METHOD_CREATE_PUSH_NOTIFICATION_CONFIG => {
            Operation::CreatePushNotificationConfig(decode(method, params)?)
        }
        METHOD_GET_PUSH_NOTIFICATION_CONFIG => {
            Operation::GetPushNotificationConfig(decode(method, params)?)
        }
        METHOD_LIST_PUSH_NOTIFICATION_CONFIGS => {
            Operation::ListPushNotificationConfigs(decode(method, params)?)
        }
        METHOD_DELETE_PUSH_NOTIFICATION_CONFIG => {
            Operation::DeletePushNotificationConfig(decode(method, params)?)
        }
        METHOD_GET_EXTENDED_AGENT_CARD => Operation::GetExtendedAgentCard,
        other => {
            return Err(A2aError::new(
                A2aErrorKind::MethodNotFound,
                format!("method not found: {other}"),
            ))
        }
    })
}

fn decode<T: DeserializeOwned>(method: &str, params: Value) -> Result<T, A2aError> {
    serde_json::from_value(params)
        .map_err(|e| A2aError::invalid_params(format!("invalid {method} params: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rejected(body: Value) -> (JsonRpcId, A2aErrorKind) {
        let (id, err) = parse_envelope(body).expect_err("rejected");
        (id, err.kind)
    }

    #[test]
    fn envelope_errors_keep_a_readable_id() {
        use A2aErrorKind::InvalidRequest;
        assert_eq!(rejected(json!([1])), (JsonRpcId::Null, InvalidRequest));
        assert_eq!(
            rejected(json!({ "jsonrpc": "2.0", "method": "GetTask" })),
            (JsonRpcId::Null, InvalidRequest)
        );
        assert_eq!(
            rejected(json!({ "jsonrpc": "2.0", "id": 1.5, "method": "GetTask" })),
            (JsonRpcId::Null, InvalidRequest)
        );
        assert_eq!(
            rejected(json!({ "jsonrpc": "2.0", "id": null, "method": "GetTask" })),
            (JsonRpcId::Null, InvalidRequest)
        );
        assert_eq!(
            rejected(json!({ "jsonrpc": "1.0", "id": 7, "method": "GetTask" })),
            (JsonRpcId::Number(7), InvalidRequest)
        );
        assert_eq!(
            rejected(json!({ "jsonrpc": "2.0", "id": "a", "method": 3 })),
            (JsonRpcId::Str("a".into()), InvalidRequest)
        );
        assert_eq!(
            rejected(json!({ "jsonrpc": "2.0", "id": 1, "method": "GetTask", "params": [1] })),
            (JsonRpcId::Number(1), InvalidRequest)
        );
        let Ok(ok) = parse_envelope(json!({ "jsonrpc": "2.0", "id": 2, "method": "ListTasks" }))
        else {
            panic!("a valid envelope was rejected");
        };
        assert_eq!(ok.method, "ListTasks");
        assert!(ok.params.is_null());
    }

    #[test]
    fn operations_decode_their_own_params_and_nothing_else() {
        assert_eq!(
            parse_operation(METHOD_LIST_TASKS, Value::Null),
            Ok(Operation::ListTasks(ListTasksRequest::default()))
        );
        assert_eq!(
            parse_operation(METHOD_GET_TASK, json!({ "id": "t1" })),
            Ok(Operation::GetTask(GetTaskRequest {
                id: "t1".into(),
                history_length: None,
            }))
        );
        assert_eq!(
            parse_operation(METHOD_GET_TASK, Value::Null)
                .err()
                .map(|e| e.kind),
            Some(A2aErrorKind::InvalidParams)
        );
        assert_eq!(
            parse_operation("message/send", json!({}))
                .err()
                .map(|e| e.kind),
            Some(A2aErrorKind::MethodNotFound)
        );
    }
}
