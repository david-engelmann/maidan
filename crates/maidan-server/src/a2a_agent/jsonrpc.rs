//! The JSON-RPC 2.0 binding (§9): `POST /a2a/v1/rpc`. Every answer is HTTP
//! 200 with a JSON-RPC response; streaming methods answer an SSE stream whose
//! events are JSON-RPC responses wrapping a `StreamResponse`.

use std::convert::Infallible;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Response, Sse};
use axum::{Extension, Json};
use futures::{Stream, StreamExt};
use maidan_a2a::{
    A2aError, A2aErrorKind, JsonRpcId, JsonRpcResponse, SendMessageResponse, StreamResponse,
    JSONRPC_VERSION, METHOD_CANCEL_TASK, METHOD_CREATE_PUSH_NOTIFICATION_CONFIG,
    METHOD_DELETE_PUSH_NOTIFICATION_CONFIG, METHOD_GET_EXTENDED_AGENT_CARD,
    METHOD_GET_PUSH_NOTIFICATION_CONFIG, METHOD_GET_TASK, METHOD_LIST_PUSH_NOTIFICATION_CONFIGS,
    METHOD_LIST_TASKS, METHOD_SEND_MESSAGE, METHOD_SEND_STREAMING_MESSAGE,
    METHOD_SUBSCRIBE_TO_TASK,
};
use maidan_auth::AuthContext;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use super::error::internal;
use super::{card, ops, push, version};
use crate::state::AppState;

/// A body the transport could not read, answered with a null id: JSON that
/// does not parse is a Parse error, a body not sent as JSON is
/// ContentTypeNotSupported, anything else an Invalid Request. A body over the
/// size limit or not sent as JSON keeps its HTTP status (413, 415).
fn rpc_rejected(status: StatusCode, detail: String) -> Response {
    let kind = match status {
        StatusCode::BAD_REQUEST => A2aErrorKind::ParseError,
        StatusCode::UNSUPPORTED_MEDIA_TYPE => A2aErrorKind::ContentTypeNotSupported,
        _ => A2aErrorKind::InvalidRequest,
    };
    let (status, detail) = crate::extract::rejection(status, detail);
    let status = if status == StatusCode::BAD_REQUEST {
        StatusCode::OK
    } else {
        status
    };
    (
        status,
        failure(JsonRpcId::Null, &A2aError::new(kind, detail)),
    )
        .into_response()
}

crate::extract::wrap_extractor!(
    /// The JSON-RPC request body, rejected as a JSON-RPC error.
    RpcJson,
    body Json,
    Response,
    rpc_rejected
);

fn failure(id: JsonRpcId, err: &A2aError) -> Json<JsonRpcResponse> {
    Json(JsonRpcResponse::failure(id, err.to_json_rpc()))
}

/// A request's envelope, read by hand so an envelope error still answers
/// with the request's id where it has a usable one.
struct Envelope {
    id: JsonRpcId,
    method: String,
    params: Value,
}

fn envelope(body: Value) -> Result<Envelope, (JsonRpcId, A2aError)> {
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

fn params<T: DeserializeOwned>(method: &str, params: Value) -> Result<T, A2aError> {
    serde_json::from_value(params)
        .map_err(|e| A2aError::invalid_params(format!("invalid {method} params: {e}")))
}

fn reply<T: Serialize>(id: JsonRpcId, result: Result<T, A2aError>) -> Response {
    let value = result.and_then(|value| serde_json::to_value(value).map_err(internal));
    match value {
        Ok(value) => Json(JsonRpcResponse::success(id, value)).into_response(),
        Err(err) => failure(id, &err).into_response(),
    }
}

fn stream<S>(id: JsonRpcId, events: Result<S, A2aError>) -> Response
where
    S: Stream<Item = StreamResponse> + Send + 'static,
{
    match events {
        Ok(events) => Sse::new(events.map(move |event| {
            let frame = match serde_json::to_value(event) {
                Ok(value) => JsonRpcResponse::success(id.clone(), value),
                Err(err) => JsonRpcResponse::failure(id.clone(), internal(err).to_json_rpc()),
            };
            let data = serde_json::to_string(&frame).unwrap_or_default();
            Ok::<Event, Infallible>(Event::default().data(data))
        }))
        .into_response(),
        Err(err) => failure(id, &err).into_response(),
    }
}

pub async fn json_rpc(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
    RpcJson(body): RpcJson<Value>,
) -> Response {
    let Envelope {
        id,
        method,
        params: p,
    } = match envelope(body) {
        Ok(envelope) => envelope,
        Err((id, err)) => return failure(id, &err).into_response(),
    };
    if let Err(err) = version::check(&headers, uri.query()) {
        return failure(id, &err).into_response();
    }
    let (state, auth) = (&state, &auth);
    match method.as_str() {
        METHOD_SEND_MESSAGE => {
            let result = match params(&method, p) {
                Ok(req) => ops::send_message(state, auth, req)
                    .await
                    .map(SendMessageResponse::Task),
                Err(err) => Err(err),
            };
            reply(id, result)
        }
        METHOD_SEND_STREAMING_MESSAGE => {
            let events = match params(&method, p) {
                Ok(req) => ops::send_streaming_message(state, auth, req)
                    .await
                    .map(futures::stream::iter),
                Err(err) => Err(err),
            };
            stream(id, events)
        }
        METHOD_GET_TASK => match params(&method, p) {
            Ok(req) => reply(id, ops::get_task(state, auth, req).await),
            Err(err) => failure(id, &err).into_response(),
        },
        METHOD_LIST_TASKS => {
            let req = if p.is_null() {
                Ok(Default::default())
            } else {
                params(&method, p)
            };
            match req {
                Ok(req) => reply(id, ops::list_tasks(state, auth, req).await),
                Err(err) => failure(id, &err).into_response(),
            }
        }
        METHOD_CANCEL_TASK => match params(&method, p) {
            Ok(req) => reply(id, ops::cancel_task(state, auth, req).await),
            Err(err) => failure(id, &err).into_response(),
        },
        METHOD_SUBSCRIBE_TO_TASK => {
            let events = match params(&method, p) {
                Ok(req) => ops::subscribe(state, auth, req).await,
                Err(err) => Err(err),
            };
            stream(id, events)
        }
        METHOD_CREATE_PUSH_NOTIFICATION_CONFIG => match params(&method, p) {
            Ok(req) => reply(id, push::create(state, auth, req).await),
            Err(err) => failure(id, &err).into_response(),
        },
        METHOD_GET_PUSH_NOTIFICATION_CONFIG => match params(&method, p) {
            Ok(req) => reply(id, push::get(state, auth, req).await),
            Err(err) => failure(id, &err).into_response(),
        },
        METHOD_LIST_PUSH_NOTIFICATION_CONFIGS => match params(&method, p) {
            Ok(req) => reply(id, push::list(state, auth, req).await),
            Err(err) => failure(id, &err).into_response(),
        },
        METHOD_DELETE_PUSH_NOTIFICATION_CONFIG => match params(&method, p) {
            Ok(req) => reply(
                id,
                push::delete(state, auth, req)
                    .await
                    .map(|()| serde_json::json!({})),
            ),
            Err(err) => failure(id, &err).into_response(),
        },
        METHOD_GET_EXTENDED_AGENT_CARD => reply(id, card::extended(state, auth)),
        other => failure(
            id,
            &A2aError::new(
                A2aErrorKind::MethodNotFound,
                format!("method not found: {other}"),
            ),
        )
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rejected(body: Value) -> (JsonRpcId, A2aErrorKind) {
        let (id, err) = envelope(body).err().expect("rejected");
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
        let Ok(ok) = envelope(json!({ "jsonrpc": "2.0", "id": 2, "method": "ListTasks" })) else {
            panic!("a valid envelope was rejected");
        };
        assert_eq!(ok.method, "ListTasks");
        assert!(ok.params.is_null());
    }
}
