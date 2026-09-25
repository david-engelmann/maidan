//! HTTP error type. `ApiError` wraps `StoreError` and any local
//! validation failures and renders as RFC 9457 `application/problem+json`
//! bodies.

use axum::{
    extract::{rejection::JsonRejection, FromRequest, Request},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use maidan_store::StoreError;
use maidan_types::{LogSnapshot, WorkspaceId};
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
                    // Preserve a body-size-limit rejection as 413; the
                    // `DefaultBodyLimit` layer surfaces it as a
                    // `PAYLOAD_TOO_LARGE` status on the rejection. Everything
                    // else is a 400.
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
    BadGateway(String),
    Internal(String),
    /// Subscribe / backfill cursor is behind the retained log. 409 +
    /// `must_refetch: true` — fail loud, never clamp. Carries a `snapshot` so
    /// the client can refetch the pruned prefix.
    CursorTooOld {
        after_id: i64,
        oldest_id: i64,
        snapshot: Option<String>,
    },
    /// Hash-chained event log is broken. 409 fail-closed.
    EventLogBroken {
        break_at: Option<i64>,
        reason: maidan_types::ChainBreakReason,
    },
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
            Self::BadGateway(_) => StatusCode::BAD_GATEWAY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::CursorTooOld { .. } | Self::EventLogBroken { .. } => StatusCode::CONFLICT,
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
            Self::BadGateway(_) => "Bad Gateway",
            Self::Internal(_) => "Internal Server Error",
            Self::CursorTooOld { .. } => "Cursor Too Old",
            Self::EventLogBroken { .. } => "Event Log Broken",
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
            | Self::BadGateway(msg)
            | Self::Internal(msg) => msg.clone(),
            Self::CursorTooOld {
                after_id,
                oldest_id,
                snapshot,
            } => {
                let mut detail = format!(
                    "subscribe cursor after_id={after_id} is behind the oldest retained event {oldest_id}; must refetch"
                );
                if let Some(path) = snapshot {
                    detail.push_str("; snapshot ");
                    detail.push_str(path);
                }
                detail
            }
            Self::EventLogBroken { break_at, reason } => match break_at {
                Some(id) => format!("event log chain broken at id={id}: {}", reason.as_str()),
                None => format!("event log chain broken: {}", reason.as_str()),
            },
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
            Self::BadGateway(_) => "https://maidan.dev/problems/bad-gateway",
            Self::Internal(_) => "https://maidan.dev/problems/internal",
            Self::CursorTooOld { .. } => "https://maidan.dev/problems/cursor-too-old",
            Self::EventLogBroken { .. } => "https://maidan.dev/problems/event-log-broken",
        }
    }

    /// Point a CursorTooOld 409 at the workspace snapshot a peer must refetch.
    /// Other errors are unchanged.
    pub fn with_snapshot(self, workspace_id: WorkspaceId) -> Self {
        match self {
            Self::CursorTooOld {
                after_id,
                oldest_id,
                ..
            } => Self::CursorTooOld {
                after_id,
                oldest_id,
                snapshot: Some(LogSnapshot::path(workspace_id)),
            },
            other => other,
        }
    }

    pub fn with_snapshot_opt(self, workspace_id: Option<WorkspaceId>) -> Self {
        match workspace_id {
            Some(ws) => self.with_snapshot(ws),
            None => self,
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

/// RFC 9457 problem details (`application/problem+json`; 9457 obsoletes 7807
/// and keeps its format).
#[derive(Debug, Serialize, ToSchema)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    #[schema(example = "https://maidan.dev/problems/not-found")]
    pub type_: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
    /// A CursorTooOld 409 is a must-refetch, never a silent clamp.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub must_refetch: bool,
    /// GET this path for a hashed snapshot covering the pruned prefix, then
    /// catch up from `as_of_lsn`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let snapshot = match &self {
            Self::CursorTooOld { snapshot, .. } => snapshot.clone(),
            _ => None,
        };
        let body = ProblemDetails {
            type_: self.problem_type().to_string(),
            title: self.title().to_string(),
            status: status.as_u16(),
            detail: self.detail(),
            must_refetch: matches!(self, Self::CursorTooOld { .. }),
            snapshot,
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
            // A refused spawn is a state conflict like any other: the typing
            // exists so the route can publish `ThreadSpawnDenied`, not to
            // change the wire status.
            StoreError::SpawnRejected(denial) => Self::Conflict(denial.to_string()),
            StoreError::CursorTooOld {
                after_id,
                oldest_id,
            } => Self::CursorTooOld {
                after_id,
                oldest_id,
                snapshot: None,
            },
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use maidan_types::{LogSnapshot, WorkspaceId};

    #[test]
    fn cursor_too_old_is_409_must_refetch() {
        let err = ApiError::from(StoreError::cursor_too_old(10, 50));
        assert_eq!(err.status(), StatusCode::CONFLICT);
        assert_eq!(
            err.problem_type(),
            "https://maidan.dev/problems/cursor-too-old"
        );
        assert!(err.detail().contains("must refetch"));
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn cursor_too_old_snapshot_href_is_workspace_scoped() {
        let ws = WorkspaceId(uuid::Uuid::from_u128(1));
        let err = ApiError::from(StoreError::cursor_too_old(10, 50)).with_snapshot(ws);
        assert!(err.detail().contains("/workspaces/"));
        assert!(err.detail().contains("/snapshot"));
        match err {
            ApiError::CursorTooOld { snapshot, .. } => {
                assert_eq!(snapshot.as_deref(), Some(LogSnapshot::path(ws).as_str()));
            }
            other => panic!("expected CursorTooOld, got {other:?}"),
        }
    }

    #[test]
    fn event_log_broken_is_409_fail_closed() {
        let err = ApiError::EventLogBroken {
            break_at: Some(4),
            reason: maidan_types::ChainBreakReason::ContentHashMismatch,
        };
        assert_eq!(err.status(), StatusCode::CONFLICT);
        assert_eq!(
            err.problem_type(),
            "https://maidan.dev/problems/event-log-broken"
        );
        assert!(err.detail().contains("id=4"));
        assert!(err.detail().contains("content_hash_mismatch"));
        assert_eq!(err.into_response().status(), StatusCode::CONFLICT);
    }
}
