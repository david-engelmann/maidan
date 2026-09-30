//! The HTTP+JSON binding (§11) under `/a2a/v1`. Successes are
//! `application/json`; errors are AIP-193 bodies with the error's HTTP status
//! (§11.6); streaming operations answer SSE whose events are `StreamResponse`
//! objects.

use std::convert::Infallible;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Response, Sse};
use axum::{Extension, Json};
use futures::{Stream, StreamExt};
use maidan_a2a::{
    A2aError, A2aErrorKind, CancelTaskRequest, DeleteTaskPushNotificationConfigRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTasksRequest, Method, SendMessageRequest, SendMessageResponse, StreamResponse,
    SubscribeToTaskRequest, TaskPushNotificationConfig,
};
use maidan_auth::AuthContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::internal;
use super::{card, ops, push, recorded, version};
use crate::state::AppState;

/// A request the binding could not read, as an AIP-193 body with the
/// extractor's status.
fn rest_rejected(status: StatusCode, detail: String) -> Response {
    let (status, detail) = crate::extract::rejection(status, detail);
    let body = serde_json::json!({ "error": {
        "code": status.as_u16(),
        "status": "INVALID_ARGUMENT",
        "message": detail,
    } });
    (status, Json(body)).into_response()
}

crate::extract::wrap_extractor!(
    /// A path parameter, rejected as an AIP-193 error.
    RestPath,
    parts Path,
    Response,
    rest_rejected
);
crate::extract::wrap_extractor!(
    /// A query string, rejected as an AIP-193 error.
    RestQuery,
    parts Query,
    Response,
    rest_rejected
);
crate::extract::wrap_extractor!(
    /// A JSON body, rejected as an AIP-193 error.
    RestJson,
    body Json,
    Response,
    rest_rejected
);

fn error(err: &A2aError) -> Response {
    let status =
        StatusCode::from_u16(err.kind.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(err.to_rest_body())).into_response()
}

fn reply<T: Serialize>(result: Result<T, A2aError>) -> Response {
    match result.and_then(|value| serde_json::to_value(value).map_err(internal)) {
        Ok(value) => Json(value).into_response(),
        Err(err) => error(&err),
    }
}

fn stream<S>(events: Result<S, A2aError>) -> Response
where
    S: Stream<Item = StreamResponse> + Send + 'static,
{
    match events {
        Ok(events) => Sse::new(events.map(|event| {
            let data = serde_json::to_string(&event).unwrap_or_else(|err| {
                serde_json::to_string(&internal(err).to_rest_body()).unwrap_or_default()
            });
            Ok::<Event, Infallible>(Event::default().data(data))
        }))
        .into_response(),
        Err(err) => error(&err),
    }
}

/// Every operation first checks the protocol version the request names.
macro_rules! versioned {
    ($headers:expr, $uri:expr) => {
        if let Err(err) = version::check(&$headers, $uri.query()) {
            return error(&err);
        }
    };
}

fn int(name: &str, value: Option<&str>) -> Result<Option<i32>, A2aError> {
    value
        .map(|v| {
            v.parse::<i32>().map_err(|_| {
                A2aError::invalid_params(format!("{name} must be an integer, got {v}"))
            })
        })
        .transpose()
}

/// `POST /message:send` and `POST /message:stream`. The path parameter is
/// the text after `message`, colon included.
pub async fn rest_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
    RestPath(method): RestPath<String>,
    RestJson(req): RestJson<SendMessageRequest>,
) -> Response {
    versioned!(headers, uri);
    match method.as_str() {
        ":send" => reply(
            recorded(
                &state,
                &auth,
                "rest",
                Method::SendMessage,
                ops::send_message(&state, &auth, req),
            )
            .await
            .map(SendMessageResponse::Task),
        ),
        ":stream" => stream(
            recorded(
                &state,
                &auth,
                "rest",
                Method::SendStreamingMessage,
                ops::send_streaming_message(&state, &auth, req),
            )
            .await
            .map(futures::stream::iter),
        ),
        other => error(&A2aError::new(
            A2aErrorKind::MethodNotFound,
            format!("unknown message method: message{other}"),
        )),
    }
}

/// `GET /tasks` query parameters, kept as text so a malformed number gets
/// the binding's error.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksQuery {
    context_id: Option<String>,
    status: Option<String>,
    page_size: Option<String>,
    page_token: Option<String>,
    history_length: Option<String>,
    status_timestamp_after: Option<String>,
    include_artifacts: Option<String>,
}

impl ListTasksQuery {
    fn request(self) -> Result<ListTasksRequest, A2aError> {
        let include_artifacts = match self.include_artifacts.as_deref() {
            None => None,
            Some("true") => Some(true),
            Some("false") => Some(false),
            Some(v) => {
                return Err(A2aError::invalid_params(format!(
                    "includeArtifacts must be true or false, got {v}"
                )))
            }
        };
        Ok(ListTasksRequest {
            context_id: self.context_id,
            status: self.status,
            page_size: int("pageSize", self.page_size.as_deref())?,
            page_token: self.page_token,
            history_length: int("historyLength", self.history_length.as_deref())?,
            status_timestamp_after: self.status_timestamp_after,
            include_artifacts,
        })
    }
}

/// `GET /tasks`
pub async fn rest_list_tasks(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
    RestQuery(query): RestQuery<ListTasksQuery>,
) -> Response {
    versioned!(headers, uri);
    match query.request() {
        Ok(req) => reply(
            recorded(
                &state,
                &auth,
                "rest",
                Method::ListTasks,
                ops::list_tasks(&state, &auth, req),
            )
            .await,
        ),
        Err(err) => error(&err),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTaskQuery {
    history_length: Option<String>,
}

/// A `/tasks/{id}` segment: a task id, or `{id}:{method}` for a custom
/// method (axum captures whole segments; task ids never contain `:`).
fn split(segment: &str) -> (&str, Option<&str>) {
    match segment.rsplit_once(':') {
        Some((id, method)) => (id, Some(method)),
        None => (segment, None),
    }
}

fn unknown_method(method: &str) -> Response {
    error(&A2aError::new(
        A2aErrorKind::MethodNotFound,
        format!("unknown task method: {method}"),
    ))
}

/// `GET /tasks/{id}` and `GET /tasks/{id}:subscribe`
pub async fn rest_task_get(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
    RestPath(segment): RestPath<String>,
    RestQuery(query): RestQuery<GetTaskQuery>,
) -> Response {
    versioned!(headers, uri);
    match split(&segment) {
        (id, None) => match int("historyLength", query.history_length.as_deref()) {
            Ok(history_length) => {
                let req = GetTaskRequest {
                    id: id.to_string(),
                    history_length,
                };
                let call = ops::get_task(&state, &auth, req);
                reply(recorded(&state, &auth, "rest", Method::GetTask, call).await)
            }
            Err(err) => error(&err),
        },
        (id, Some("subscribe")) => subscribe(&state, &auth, id).await,
        (_, Some(method)) => unknown_method(method),
    }
}

/// `SubscribeToTask`, served on both `GET` and `POST /tasks/{id}:subscribe`.
async fn subscribe(state: &AppState, auth: &AuthContext, id: &str) -> Response {
    let call = ops::subscribe(state, auth, SubscribeToTaskRequest { id: id.to_string() });
    stream(recorded(state, auth, "rest", Method::SubscribeToTask, call).await)
}

/// `POST /tasks/{id}:cancel` and `POST /tasks/{id}:subscribe`
pub async fn rest_task_post(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
    RestPath(segment): RestPath<String>,
) -> Response {
    versioned!(headers, uri);
    match split(&segment) {
        (id, Some("cancel")) => {
            let req = CancelTaskRequest {
                id: id.to_string(),
                metadata: None,
            };
            let call = ops::cancel_task(&state, &auth, req);
            reply(recorded(&state, &auth, "rest", Method::CancelTask, call).await)
        }
        (id, Some("subscribe")) => subscribe(&state, &auth, id).await,
        (_, Some(method)) => unknown_method(method),
        (_, None) => error(&A2aError::new(
            A2aErrorKind::MethodNotFound,
            "POST needs a task method: {id}:cancel or {id}:subscribe",
        )),
    }
}

/// `POST /tasks/{id}/pushNotificationConfigs`: the body is the config; its
/// `taskId`, if any, must match the path.
pub async fn rest_create_push_config(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
    RestPath(task_id): RestPath<String>,
    RestJson(mut body): RestJson<Value>,
) -> Response {
    versioned!(headers, uri);
    let Some(fields) = body.as_object_mut() else {
        return error(&A2aError::invalid_params(
            "the body must be a push notification config",
        ));
    };
    match fields.get("taskId").and_then(Value::as_str) {
        Some(named) if named != task_id => {
            return error(&A2aError::invalid_params(
                "the body's taskId does not match the path",
            ))
        }
        _ => {}
    }
    fields.insert("taskId".into(), Value::String(task_id));
    match serde_json::from_value::<TaskPushNotificationConfig>(body) {
        Ok(config) => {
            let call = push::create(&state, &auth, config);
            let method = Method::CreatePushNotificationConfig;
            reply(recorded(&state, &auth, "rest", method, call).await)
        }
        Err(err) => error(&A2aError::invalid_params(format!(
            "invalid push notification config: {err}"
        ))),
    }
}

/// `GET /tasks/{id}/pushNotificationConfigs`
pub async fn rest_list_push_configs(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
    RestPath(task_id): RestPath<String>,
    RestQuery(query): RestQuery<ListPushConfigsQuery>,
) -> Response {
    versioned!(headers, uri);
    let page_size = match int("pageSize", query.page_size.as_deref()) {
        Ok(page_size) => page_size,
        Err(err) => return error(&err),
    };
    let req = ListTaskPushNotificationConfigsRequest {
        task_id,
        page_size,
        page_token: query.page_token,
    };
    let call = push::list(&state, &auth, req);
    let method = Method::ListPushNotificationConfigs;
    reply(recorded(&state, &auth, "rest", method, call).await)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPushConfigsQuery {
    page_size: Option<String>,
    page_token: Option<String>,
}

/// `GET /tasks/{id}/pushNotificationConfigs/{configId}`
pub async fn rest_get_push_config(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
    RestPath((task_id, id)): RestPath<(String, String)>,
) -> Response {
    versioned!(headers, uri);
    let req = GetTaskPushNotificationConfigRequest { task_id, id };
    let call = push::get(&state, &auth, req);
    let method = Method::GetPushNotificationConfig;
    reply(recorded(&state, &auth, "rest", method, call).await)
}

/// `DELETE /tasks/{id}/pushNotificationConfigs/{configId}`
pub async fn rest_delete_push_config(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
    RestPath((task_id, id)): RestPath<(String, String)>,
) -> Response {
    versioned!(headers, uri);
    let req = DeleteTaskPushNotificationConfigRequest { task_id, id };
    let call = push::delete(&state, &auth, req);
    let method = Method::DeletePushNotificationConfig;
    reply(
        recorded(&state, &auth, "rest", method, call)
            .await
            .map(|()| serde_json::json!({})),
    )
}

/// `GET /extendedAgentCard`
pub async fn rest_extended_agent_card(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    versioned!(headers, uri);
    let call = std::future::ready(card::extended(&state, &auth));
    reply(recorded(&state, &auth, "rest", Method::GetExtendedAgentCard, call).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_methods_split_off_the_last_colon() {
        assert_eq!(split("abc"), ("abc", None));
        assert_eq!(split("abc:cancel"), ("abc", Some("cancel")));
        assert_eq!(split("abc:subscribe"), ("abc", Some("subscribe")));
    }

    #[test]
    fn list_query_parses_or_refuses() {
        let req = ListTasksQuery {
            page_size: Some("10".into()),
            include_artifacts: Some("true".into()),
            ..Default::default()
        }
        .request()
        .unwrap();
        assert_eq!(req.page_size, Some(10));
        assert_eq!(req.include_artifacts, Some(true));
        for bad in [
            ListTasksQuery {
                page_size: Some("ten".into()),
                ..Default::default()
            },
            ListTasksQuery {
                include_artifacts: Some("yes".into()),
                ..Default::default()
            },
        ] {
            assert_eq!(bad.request().unwrap_err().kind, A2aErrorKind::InvalidParams);
        }
    }
}
