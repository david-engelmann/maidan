//! HTTP error type. `ApiError` wraps `StoreError` and any local
//! validation failures and renders as RFC 7807 `application/problem+json`
//! bodies.

use axum::{
    extract::{rejection::JsonRejection, FromRequest, Request},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use maidan_store::StoreError;
use serde::{de::DeserializeOwned, Serialize};
use utoipa::ToSchema;

/// Custom JSON extractor that maps deserialization errors to
/// [`ApiError::BadRequest`] so the response is `application/problem+json`
/// instead of axum's default text/plain.
pub struct ApiJson<T>(pub T);

#[axum::async_trait]
impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let Json(value) =
            Json::<T>::from_request(req, state)
                .await
                .map_err(|e: JsonRejection| {
                    // Preserve a body-size-limit rejection as 413 (Cluster 183); the
                    // `DefaultBodyLimit` layer surfaces it as a `PAYLOAD_TOO_LARGE`
                    // status on the rejection. Everything else is a 400.
                    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                        ApiError::PayloadTooLarge(e.body_text())
                    } else {
                        ApiError::BadRequest(e.body_text())
                    }
                })?;
        Ok(Self(value))
    }
}

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    Conflict(String),
    BadRequest(String),
    Unauthorized,
    Forbidden(String),
    PayloadTooLarge(String),
    TooManyRequests(String),
    Internal(String),
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::NotFound => "Not Found",
            Self::Conflict(_) => "Conflict",
            Self::BadRequest(_) => "Bad Request",
            Self::Unauthorized => "Unauthorized",
            Self::Forbidden(_) => "Forbidden",
            Self::PayloadTooLarge(_) => "Payload Too Large",
            Self::TooManyRequests(_) => "Too Many Requests",
            Self::Internal(_) => "Internal Server Error",
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::NotFound => "the requested resource does not exist".to_string(),
            Self::Unauthorized => "missing or invalid bearer token".to_string(),
            Self::Conflict(msg)
            | Self::BadRequest(msg)
            | Self::Forbidden(msg)
            | Self::PayloadTooLarge(msg)
            | Self::TooManyRequests(msg)
            | Self::Internal(msg) => msg.clone(),
        }
    }

    fn problem_type(&self) -> &'static str {
        match self {
            Self::NotFound => "https://maidan.dev/problems/not-found",
            Self::Conflict(_) => "https://maidan.dev/problems/conflict",
            Self::BadRequest(_) => "https://maidan.dev/problems/bad-request",
            Self::Unauthorized => "https://maidan.dev/problems/unauthorized",
            Self::Forbidden(_) => "https://maidan.dev/problems/forbidden",
            Self::PayloadTooLarge(_) => "https://maidan.dev/problems/payload-too-large",
            Self::TooManyRequests(_) => "https://maidan.dev/problems/rate-limited",
            Self::Internal(_) => "https://maidan.dev/problems/internal",
        }
    }
}

impl From<maidan_auth::AuthError> for ApiError {
    fn from(err: maidan_auth::AuthError) -> Self {
        use maidan_auth::AuthError;
        match err {
            AuthError::Unauthorized => Self::Unauthorized,
            AuthError::Forbidden(msg) => Self::Forbidden(msg),
            AuthError::Store(e) => e.into(),
        }
    }
}

/// RFC 7807 problem details (`application/problem+json`).
#[derive(Debug, Serialize, ToSchema)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    #[schema(example = "https://maidan.dev/problems/not-found")]
    pub type_: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ProblemDetails {
            type_: self.problem_type().to_string(),
            title: self.title().to_string(),
            status: status.as_u16(),
            detail: self.detail(),
        };
        let mut response = (status, Json(body)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

impl From<maidan_router::RouterError> for ApiError {
    fn from(err: maidan_router::RouterError) -> Self {
        match err {
            maidan_router::RouterError::Store(e) => e.into(),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::NotFound => Self::NotFound,
            StoreError::Conflict(msg) => Self::Conflict(msg),
            StoreError::InvalidInput(msg) => Self::BadRequest(msg),
            StoreError::Database(e) => {
                tracing::error!(error = %e, "database error");
                Self::Internal("database error".into())
            }
            StoreError::Migration(e) => {
                tracing::error!(error = %e, "migration error");
                Self::Internal("migration error".into())
            }
            StoreError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error");
                Self::Internal("serialization error".into())
            }
        }
    }
}

impl From<maidan_artifacts::ArtifactError> for ApiError {
    fn from(err: maidan_artifacts::ArtifactError) -> Self {
        use maidan_artifacts::ArtifactError;
        match err {
            ArtifactError::NotFound => Self::NotFound,
            ArtifactError::InvalidSha(msg) => Self::BadRequest(msg),
            ArtifactError::InvalidInput(msg) => Self::BadRequest(msg),
            ArtifactError::Io(e) => {
                tracing::error!(error = %e, "artifact io error");
                Self::Internal("artifact storage error".into())
            }
            ArtifactError::Storage(msg) => {
                tracing::error!(error = %msg, "artifact backend error");
                Self::Internal("artifact storage error".into())
            }
        }
    }
}

impl From<maidan_search::SearchError> for ApiError {
    fn from(err: maidan_search::SearchError) -> Self {
        use maidan_search::SearchError;
        match err {
            SearchError::InvalidQuery(msg) => Self::BadRequest(msg),
            SearchError::Unsupported(feature) => {
                Self::BadRequest(format!("not supported by backend: {feature}"))
            }
            SearchError::Database(e) => {
                tracing::error!(error = %e, "search database error");
                Self::Internal("search database error".into())
            }
        }
    }
}

impl From<axum::extract::rejection::JsonRejection> for ApiError {
    fn from(rej: axum::extract::rejection::JsonRejection) -> Self {
        Self::BadRequest(rej.body_text())
    }
}

impl From<axum::extract::rejection::PathRejection> for ApiError {
    fn from(rej: axum::extract::rejection::PathRejection) -> Self {
        Self::BadRequest(rej.body_text())
    }
}
