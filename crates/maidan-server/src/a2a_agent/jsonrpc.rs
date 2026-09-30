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
    Method, Operation, SendMessageResponse, StreamResponse,
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

fn reply<T: Serialize>(id: &JsonRpcId, value: T) -> Result<Response, A2aError> {
    let value = serde_json::to_value(value).map_err(internal)?;
    Ok(Json(JsonRpcResponse::success(id.clone(), value)).into_response())
}

fn stream<S>(id: &JsonRpcId, events: S) -> Response
where
    S: Stream<Item = StreamResponse> + Send + 'static,
{
    let id = id.clone();
    Sse::new(events.map(move |event| {
        let frame = match serde_json::to_value(event) {
            Ok(value) => JsonRpcResponse::success(id.clone(), value),
            Err(err) => JsonRpcResponse::failure(id.clone(), internal(err).to_json_rpc()),
        };
        let data = serde_json::to_string(&frame).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().data(data))
    }))
    .into_response()
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
    let method = operation.method();
    let call = dispatch(&state, &auth, &id, operation);
    match recorded(&state, &auth, "jsonrpc", method, call).await {
        Ok(response) => response,
        Err(err) => failure(id, &err).into_response(),
    }
}

async fn dispatch(
    state: &AppState,
    auth: &AuthContext,
    id: &JsonRpcId,
    operation: Operation,
) -> Result<Response, A2aError> {
    match operation {
        Operation::SendMessage(req) => {
            let task = ops::send_message(state, auth, req).await?;
            reply(id, SendMessageResponse::Task(task))
        }
        Operation::SendStreamingMessage(req) => {
            let events = ops::send_streaming_message(state, auth, req).await?;
            Ok(stream(id, futures::stream::iter(events)))
        }
        Operation::GetTask(req) => reply(id, ops::get_task(state, auth, req).await?),
        Operation::ListTasks(req) => reply(id, ops::list_tasks(state, auth, req).await?),
        Operation::CancelTask(req) => reply(id, ops::cancel_task(state, auth, req).await?),
        Operation::SubscribeToTask(req) => Ok(stream(id, ops::subscribe(state, auth, req).await?)),
        Operation::CreatePushNotificationConfig(req) => {
            reply(id, push::create(state, auth, req).await?)
        }
        Operation::GetPushNotificationConfig(req) => reply(id, push::get(state, auth, req).await?),
        Operation::ListPushNotificationConfigs(req) => {
            reply(id, push::list(state, auth, req).await?)
        }
        Operation::DeletePushNotificationConfig(req) => {
            push::delete(state, auth, req).await?;
            reply(id, serde_json::json!({}))
        }
        Operation::GetExtendedAgentCard => reply(id, card::extended(state, auth)?),
    }
}

/// Run one A2A method as its caller, and make sure a change leaves a record:
/// the A2A half of the rule `auth::run_as` enforces over REST, kept per method
/// because one endpoint serves them all and a JSON-RPC error still answers
/// HTTP 200. The request layer, which saw only a successful POST, recorded
/// `ListTasks` as a change. A call that fails records nothing; a method that
/// [changes](Method::changes) and succeeds without writing an event or audit
/// row of its own gets a `mutation` row naming the method. Every binding runs
/// every method through here, and gRPC, which has no request layer, takes its
/// caller's attribution from it.
pub(crate) async fn recorded<T, E>(
    state: &AppState,
    auth: &AuthContext,
    binding: &'static str,
    method: Method,
    call: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let attribution = auth.attribution();
    let (result, recorded) =
        maidan_store::attribution::with_attribution_tracked(attribution, call).await;
    if attribution.is_some() && result.is_ok() && !recorded && method.changes() {
        maidan_store::attribution::with_attribution(
            attribution,
            crate::audit::record(
                state,
                maidan_types::NewAuditEvent {
                    scope: maidan_types::AuditScope::Workspace(auth.workspace_id),
                    actor_id: Some(auth.actor_id),
                    action: crate::auth::MUTATION_ACTION.into(),
                    target_kind: Some("workspace".into()),
                    target_id: Some(auth.workspace_id.0),
                    metadata: serde_json::json!({
                        "surface": "a2a",
                        "binding": binding,
                        "operation": method.name(),
                    }),
                },
            ),
        )
        .await;
    }
    result
}
