//! Request extractors whose rejections are RFC 9457 problems.
//!
//! axum's own `Path`, `Query`, `Json`, `Bytes` and `String` extractors reject
//! a request with a `text/plain` body. Every HTTP handler takes its input
//! through these wrappers instead, so a malformed path parameter, query
//! string or body, a body that is not JSON, or one over the body-size limit
//! (`MAIDAN_MAX_BODY_BYTES`) answers with `application/problem+json` like any
//! other client error. `crate::routing` refuses, at compile time, a route
//! whose handler takes a raw axum extractor.
//!
//! | Rejection | Status |
//! |---|---|
//! | path parameter or query string that does not deserialize | 400 |
//! | JSON body that does not parse or does not match its type | 400 |
//! | body without a JSON `Content-Type` where JSON is required | 415 |
//! | body over the body-size limit | 413 |
//! | text body that is not UTF-8 | 400 |

use axum::{
    body::Bytes,
    extract::{Form, FromRequest, Path, Query, Request},
    http::StatusCode,
    Json,
};

use crate::error::ApiError;

/// Declare an extractor that wraps axum's generic `$inner` extractor (`Path`,
/// `Query` or `Json`) and answers its rejection with `$reject(status, detail)`,
/// where `status` and `detail` are axum's. A protocol with its own error
/// envelope (SCIM, A2A) declares its extractors with this too.
macro_rules! wrap_extractor {
    ($(#[$doc:meta])* $name:ident, parts $inner:ident, $rejection:ty, $reject:expr) => {
        $(#[$doc])*
        pub struct $name<T>(pub T);

        impl<T> $crate::routing::Checked for $name<T> {}

        impl<T, S> axum::extract::FromRequestParts<S> for $name<T>
        where
            T: serde::de::DeserializeOwned + Send,
            S: Send + Sync,
        {
            type Rejection = $rejection;

            async fn from_request_parts(
                parts: &mut axum::http::request::Parts,
                state: &S,
            ) -> Result<Self, $rejection> {
                $inner::<T>::from_request_parts(parts, state)
                    .await
                    .map(|$inner(value)| Self(value))
                    .map_err(|e| $reject(e.status(), e.body_text()))
            }
        }
    };
    ($(#[$doc:meta])* $name:ident, body $inner:ident, $rejection:ty, $reject:expr) => {
        $(#[$doc])*
        pub struct $name<T>(pub T);

        impl<T> $crate::routing::Checked for $name<T> {}

        impl<T, S> axum::extract::FromRequest<S> for $name<T>
        where
            T: serde::de::DeserializeOwned,
            S: Send + Sync,
        {
            type Rejection = $rejection;

            async fn from_request(
                req: axum::extract::Request,
                state: &S,
            ) -> Result<Self, $rejection> {
                $inner::<T>::from_request(req, state)
                    .await
                    .map(|$inner(value)| Self(value))
                    .map_err(|e| $reject(e.status(), e.body_text()))
            }
        }
    };
}
pub(crate) use wrap_extractor;

/// An axum rejection's status and detail as the server reports them. A body
/// whose shape is wrong (axum's 422) is a 400 like one that does not parse:
/// there is one status for a request that cannot be read. A 5xx is a routing
/// bug, not the client's; it is logged and reported without axum's detail.
pub(crate) fn rejection(status: StatusCode, detail: String) -> (StatusCode, String) {
    if status.is_server_error() {
        tracing::error!(%status, detail, "request extractor failed");
        return (status, "request extraction failed".into());
    }
    if status == StatusCode::UNPROCESSABLE_ENTITY {
        return (StatusCode::BAD_REQUEST, detail);
    }
    (status, detail)
}

/// The problem the API answers an axum rejection with.
fn rejected(status: StatusCode, detail: String) -> ApiError {
    match rejection(status, detail) {
        (StatusCode::PAYLOAD_TOO_LARGE, detail) => ApiError::PayloadTooLarge(detail),
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, detail) => ApiError::UnsupportedMediaType(detail),
        (status, detail) if status.is_client_error() => ApiError::BadRequest(detail),
        (_, detail) => ApiError::Internal(detail),
    }
}

/// The raw request body, within the body-size limit.
pub struct ApiBytes(pub Bytes);

impl crate::routing::Checked for ApiBytes {}

impl<S> FromRequest<S> for ApiBytes
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        Bytes::from_request(req, state)
            .await
            .map(Self)
            .map_err(|e| rejected(e.status(), e.body_text()))
    }
}

/// The request body as UTF-8 text, within the body-size limit.
pub struct ApiText(pub String);

impl crate::routing::Checked for ApiText {}

impl<S> FromRequest<S> for ApiText
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        String::from_request(req, state)
            .await
            .map(Self)
            .map_err(|e| rejected(e.status(), e.body_text()))
    }
}

wrap_extractor!(
    /// The matched route's path parameters.
    ApiPath,
    parts Path,
    ApiError,
    rejected
);
wrap_extractor!(
    /// The query string.
    ApiQuery,
    parts Query,
    ApiError,
    rejected
);
wrap_extractor!(
    /// A JSON request body. Requires a JSON `Content-Type`.
    ApiJson,
    body Json,
    ApiError,
    rejected
);
wrap_extractor!(
    /// An `application/x-www-form-urlencoded` body, as the OAuth token
    /// endpoint takes. Requires the form content type.
    ApiForm,
    body Form,
    ApiError,
    rejected
);

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // mock servers, not the API
mod tests {
    use axum::{
        body::Body,
        http::{header, Request as HttpRequest},
        response::IntoResponse,
        routing::post,
        Router,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    /// A number, as a query string or a JSON body.
    #[derive(serde::Deserialize)]
    struct Number {
        n: i64,
    }

    /// Adds the query's number to the body's.
    async fn add(
        ApiPath(_): ApiPath<uuid::Uuid>,
        ApiQuery(q): ApiQuery<Number>,
        ApiJson(b): ApiJson<Number>,
    ) -> Json<i64> {
        Json(q.n + b.n)
    }

    fn app() -> Router {
        Router::new()
            .route("/{id}", post(add))
            .layer(axum::extract::DefaultBodyLimit::max(64))
    }

    async fn send(
        uri: &str,
        content_type: Option<&str>,
        body: &str,
    ) -> (StatusCode, String, Value) {
        let mut request = HttpRequest::post(uri);
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let response = app()
            .oneshot(request.body(Body::from(body.to_owned())).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (
            status,
            content_type,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    const ID: &str = "00000000-0000-0000-0000-000000000001";

    async fn assert_problem(uri: &str, content_type: Option<&str>, body: &str, status: StatusCode) {
        let (got, content_type, problem) = send(uri, content_type, body).await;
        assert_eq!(got, status, "{uri} {body}: {problem}");
        assert_eq!(content_type, "application/problem+json", "{uri} {body}");
        assert_eq!(problem["status"], status.as_u16(), "{problem}");
        assert!(
            problem["detail"].as_str().is_some_and(|d| !d.is_empty()),
            "{problem}"
        );
    }

    #[tokio::test]
    async fn a_request_every_extractor_accepts_reaches_the_handler() {
        let (status, _, sum) = send(
            &format!("/{ID}?n=1"),
            Some("application/json"),
            r#"{"n":2}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(sum, 3);
    }

    #[tokio::test]
    async fn each_rejection_is_a_problem_with_its_status() {
        let json = Some("application/json");
        let ok_body = r#"{"n":1}"#;
        assert_problem("/not-a-uuid?n=1", json, ok_body, StatusCode::BAD_REQUEST).await;
        assert_problem(
            &format!("/{ID}?n=x"),
            json,
            ok_body,
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert_problem(&format!("/{ID}"), json, ok_body, StatusCode::BAD_REQUEST).await;
        assert_problem(
            &format!("/{ID}?n=1"),
            json,
            "{not json",
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert_problem(
            &format!("/{ID}?n=1"),
            json,
            r#"{"n":"x"}"#,
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert_problem(
            &format!("/{ID}?n=1"),
            None,
            ok_body,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        )
        .await;
        assert_problem(
            &format!("/{ID}?n=1"),
            Some("text/plain"),
            ok_body,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        )
        .await;
        let big = format!(r#"{{"n":1,"pad":"{}"}}"#, "x".repeat(128));
        assert_problem(
            &format!("/{ID}?n=1"),
            json,
            &big,
            StatusCode::PAYLOAD_TOO_LARGE,
        )
        .await;
    }

    #[test]
    fn a_server_side_rejection_hides_axums_detail() {
        let (status, detail) = rejection(
            StatusCode::INTERNAL_SERVER_ERROR,
            "route has no params".into(),
        );
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(detail, "request extraction failed");
        assert_eq!(
            rejected(StatusCode::INTERNAL_SERVER_ERROR, "x".into())
                .into_response()
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
