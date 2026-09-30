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
    parse_envelope, parse_operation, A2aError, A2aErrorKind, Envelope, JsonRpcId, JsonRpcResponse,
    Operation, SendMessageResponse, StreamResponse,
};
use maidan_auth::AuthContext;
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
    let Envelope { id, method, params } = match parse_envelope(body) {
        Ok(envelope) => envelope,
        Err((id, err)) => return failure(id, &err).into_response(),
    };
    if let Err(err) = version::check(&headers, uri.query()) {
        return failure(id, &err).into_response();
    }
    let operation = match parse_operation(&method, params) {
        Ok(operation) => operation,
        Err(err) => return failure(id, &err).into_response(),
    };
    let (state, auth) = (&state, &auth);
    match operation {
        Operation::SendMessage(req) => reply(
            id,
            ops::send_message(state, auth, req)
                .await
                .map(SendMessageResponse::Task),
        ),
        Operation::SendStreamingMessage(req) => stream(
            id,
            ops::send_streaming_message(state, auth, req)
                .await
                .map(futures::stream::iter),
        ),
        Operation::GetTask(req) => reply(id, ops::get_task(state, auth, req).await),
        Operation::ListTasks(req) => reply(id, ops::list_tasks(state, auth, req).await),
        Operation::CancelTask(req) => reply(id, ops::cancel_task(state, auth, req).await),
        Operation::SubscribeToTask(req) => stream(id, ops::subscribe(state, auth, req).await),
        Operation::CreatePushNotificationConfig(req) => {
            reply(id, push::create(state, auth, req).await)
        }
        Operation::GetPushNotificationConfig(req) => reply(id, push::get(state, auth, req).await),
        Operation::ListPushNotificationConfigs(req) => {
            reply(id, push::list(state, auth, req).await)
        }
        Operation::DeletePushNotificationConfig(req) => reply(
            id,
            push::delete(state, auth, req)
                .await
                .map(|()| serde_json::json!({})),
        ),
        Operation::GetExtendedAgentCard => reply(id, card::extended(state, auth)),
    }
}
