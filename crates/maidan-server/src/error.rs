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
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|e: JsonRejection| ApiError::BadRequest(e.body_text()))?;
        Ok(Self(value))
    }
}

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    Conflict(String),
    BadRequest(String),
    Internal(String),
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::NotFound => "Not Found",
            Self::Conflict(_) => "Conflict",
            Self::BadRequest(_) => "Bad Request",
            Self::Internal(_) => "Internal Server Error",
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::NotFound => "the requested resource does not exist".to_string(),
            Self::Conflict(msg) | Self::BadRequest(msg) | Self::Internal(msg) => msg.clone(),
        }
    }

    fn problem_type(&self) -> &'static str {
        match self {
            Self::NotFound => "https://maidan.dev/problems/not-found",
            Self::Conflict(_) => "https://maidan.dev/problems/conflict",
            Self::BadRequest(_) => "https://maidan.dev/problems/bad-request",
            Self::Internal(_) => "https://maidan.dev/problems/internal",
        }
    }
}

#[derive(Debug, Serialize)]
struct ProblemBody<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    title: &'a str,
    status: u16,
    detail: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ProblemBody {
            type_: self.problem_type(),
            title: self.title(),
            status: status.as_u16(),
            detail: self.detail(),
        };
        let mut response = (status, Json(body)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            "application/problem+json".parse().unwrap(),
        );
        response
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
