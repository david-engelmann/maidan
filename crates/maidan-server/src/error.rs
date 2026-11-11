//! HTTP error type. `ApiError` wraps `StoreError`, local validation failures
//! and request-extraction rejections (`crate::extract`) and renders as RFC 9457
//! `application/problem+json` bodies.

use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use maidan_store::StoreError;
use maidan_types::{LogSnapshot, WorkspaceId};
use serde::Serialize;
use utoipa::ToSchema;

/// What a client is told about any 500. The cause (`ApiError::Internal`'s
/// text, often a library error's `to_string()`: a path, a driver message, a
/// config name) goes to the server log under the request's X-Request-Id
/// instead, so a 500 never describes the server's insides (OPS-004).
pub const INTERNAL_DETAIL: &str = "the server failed while handling this request; the X-Request-Id response header names it in the server log";

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    /// The route exists but does not take the request's method.
    MethodNotAllowed,
    Conflict(String),
    BadRequest(String),
    Unauthorized,
    /// A webhook ingress whose signature does not verify against its secret.
    SignatureInvalid,
    Forbidden(String),
    PayloadTooLarge(String),
    UnsupportedMediaType(String),
    TooManyRequests(String),
    /// An open sign-up workspace is past its free allowance on a meter this
    /// month: 402, naming the meter and how to top up. Not a 429: waiting
    /// does not help, paying (or freeing space, for `stored_bytes`) does.
    PaymentRequired(maidan_types::UsageMeter),
    BadGateway(String),
    Internal(String),
    /// The server is at a capacity ceiling (in-flight requests, subscriber
    /// connections); retry later. 503.
    Overloaded(String),
    /// An `Idempotency-Key` reused for a different request. 422.
    IdempotencyKeyReused,
    /// The request parsed but a field value is invalid (a blank title, a
    /// negative limit). 422: the shape was fine, the value was not.
    UnprocessableEntity(String),
    /// A retry arrived while the first request with its `Idempotency-Key` is
    /// still running. 409 with its own type, so a client can tell "retry
    /// shortly" from a conflict with the resource's state.
    IdempotencyKeyInFlight,
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
    /// A media upload would take the workspace past its media quota. 409
    /// with the three numbers, so a client can tell how much to free.
    MediaQuotaExceeded {
        used_bytes: i64,
        quota_bytes: i64,
        upload_bytes: i64,
    },
    /// A footprint set on a claimed thread overlaps other live claims. 409
    /// with each overlapped claim item and its lease, so a client can defer
    /// until the lease would end.
    FootprintConflict(Vec<maidan_types::FootprintConflict>),
    /// A plan parent already has children from another claim or with other
    /// keys. 409 with the children it has, so the planner can work with them.
    PlanConflict(Vec<maidan_types::PlannedChild>),
    /// A claim-fenced write (a thread result) named a lease the thread no
    /// longer holds: it lapsed and was taken over, or was released. 409 with
    /// its own type, so a seat stops the work rather than retrying.
    ClaimLost(String),
    /// A write aimed at a thread that is closed or archived, which this write
    /// does not take. 409 with its own type, so a client stops rather than
    /// retrying.
    ThreadEnded(String),
    /// A write aimed at a thread that is gone (tombstoned). 410.
    Gone(String),
    /// A thread write refused because another open thread in the workspace
    /// gives the same values for one of its unique key sets. 409 `conflict`
    /// naming the set in `key_set`, and the holder in `holder_thread_id` only
    /// when the caller may read that thread.
    AttributeHeld {
        detail: String,
        key_set: Vec<maidan_types::UniqueAttributeKey>,
        holder: Option<maidan_types::ThreadId>,
    },
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized | Self::SignatureInvalid => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::PaymentRequired(_) => StatusCode::PAYMENT_REQUIRED,
            Self::BadGateway(_) => StatusCode::BAD_GATEWAY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Overloaded(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::IdempotencyKeyReused => StatusCode::UNPROCESSABLE_ENTITY,
            Self::UnprocessableEntity(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::IdempotencyKeyInFlight => StatusCode::CONFLICT,
            Self::CursorTooOld { .. } | Self::EventLogBroken { .. } => StatusCode::CONFLICT,
            Self::MediaQuotaExceeded { .. } => StatusCode::CONFLICT,
            Self::FootprintConflict(_) => StatusCode::CONFLICT,
            Self::PlanConflict(_) => StatusCode::CONFLICT,
            Self::ClaimLost(_) => StatusCode::CONFLICT,
            Self::ThreadEnded(_) => StatusCode::CONFLICT,
            Self::Gone(_) => StatusCode::GONE,
            Self::AttributeHeld { .. } => StatusCode::CONFLICT,
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::Conflict(_) | Self::AttributeHeld { .. } => "Conflict",
            Self::BadRequest(_) => "Bad Request",
            Self::Unauthorized | Self::SignatureInvalid => "Unauthorized",
            Self::Forbidden(_) => "Forbidden",
            Self::PayloadTooLarge(_) => "Payload Too Large",
            Self::UnsupportedMediaType(_) => "Unsupported Media Type",
            Self::TooManyRequests(_) => "Too Many Requests",
            Self::PaymentRequired(_) => "Payment Required",
            Self::BadGateway(_) => "Bad Gateway",
            Self::Internal(_) => "Internal Server Error",
            Self::Overloaded(_) => "Service Unavailable",
            Self::IdempotencyKeyReused => "Idempotency Key Reused",
            Self::UnprocessableEntity(_) => "Unprocessable Entity",
            Self::IdempotencyKeyInFlight => "Idempotency Key In Flight",
            Self::CursorTooOld { .. } => "Cursor Too Old",
            Self::EventLogBroken { .. } => "Event Log Broken",
            Self::MediaQuotaExceeded { .. } => "Media Quota Exceeded",
            Self::FootprintConflict(_) => "Footprint Conflict",
            Self::PlanConflict(_) => "Plan Conflict",
            Self::ClaimLost(_) => "Claim Lost",
            Self::ThreadEnded(_) => "Thread Ended",
            Self::Gone(_) => "Gone",
        }
    }

    pub(crate) fn detail(&self) -> String {
        match self {
            Self::NotFound => {
                "the requested resource does not exist; GET /openapi.json lists the routes"
                    .to_string()
            }
            Self::MethodNotAllowed => {
                "the route does not accept this method; see the Allow header".to_string()
            }
            Self::Unauthorized => "missing or invalid bearer token".to_string(),
            Self::SignatureInvalid => "the request signature does not verify".to_string(),
            Self::PaymentRequired(meter) => maidan_types::over_allowance_detail(*meter),
            Self::IdempotencyKeyReused => {
                "this Idempotency-Key was used for a different request; use a new key".to_string()
            }
            Self::IdempotencyKeyInFlight => {
                "a request with this Idempotency-Key is still in progress; retry shortly"
                    .to_string()
            }
            Self::Conflict(msg)
            | Self::BadRequest(msg)
            | Self::Forbidden(msg)
            | Self::PayloadTooLarge(msg)
            | Self::UnsupportedMediaType(msg)
            | Self::TooManyRequests(msg)
            | Self::BadGateway(msg)
            | Self::UnprocessableEntity(msg)
            | Self::ClaimLost(msg)
            | Self::ThreadEnded(msg)
            | Self::Gone(msg)
            | Self::Overloaded(msg)
            | Self::AttributeHeld { detail: msg, .. } => msg.clone(),
            Self::Internal(_) => INTERNAL_DETAIL.to_string(),
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
            Self::MediaQuotaExceeded {
                used_bytes,
                quota_bytes,
                upload_bytes,
            } => format!(
                "this upload of {upload_bytes} bytes would take the workspace's media \
                 to {} bytes, past its quota of {quota_bytes} ({used_bytes} used). \
                 Erase media it no longer needs, or raise the quota.",
                used_bytes.saturating_add(*upload_bytes)
            ),
            Self::FootprintConflict(conflicts) => {
                let held: std::collections::BTreeSet<_> =
                    conflicts.iter().map(|c| c.thread_id.0).collect();
                format!(
                    "the footprint overlaps {} live claim(s); nothing was stored. \
                     Release this claim and requeue, or narrow the footprint",
                    held.len()
                )
            }
            Self::PlanConflict(children) => format!(
                "this plan already has {} children, from another claim or with other keys; \
                 nothing was created. Work with the children listed, or ask a person to replan",
                children.len()
            ),
        }
    }

    fn problem_type(&self) -> &'static str {
        match self {
            Self::NotFound => "https://maidan.world/problems/not-found",
            Self::MethodNotAllowed => "https://maidan.world/problems/method-not-allowed",
            Self::Conflict(_) | Self::AttributeHeld { .. } => {
                "https://maidan.world/problems/conflict"
            }
            Self::BadRequest(_) => "https://maidan.world/problems/bad-request",
            Self::Unauthorized => "https://maidan.world/problems/unauthorized",
            Self::SignatureInvalid => "https://maidan.world/problems/invalid-signature",
            Self::Forbidden(_) => "https://maidan.world/problems/forbidden",
            Self::PayloadTooLarge(_) => "https://maidan.world/problems/payload-too-large",
            Self::UnsupportedMediaType(_) => "https://maidan.world/problems/unsupported-media-type",
            Self::TooManyRequests(_) => "https://maidan.world/problems/rate-limited",
            Self::PaymentRequired(_) => "https://maidan.world/problems/payment-required",
            Self::BadGateway(_) => "https://maidan.world/problems/bad-gateway",
            Self::Internal(_) => "https://maidan.world/problems/internal",
            Self::Overloaded(_) => "https://maidan.world/problems/overloaded",
            Self::IdempotencyKeyReused => "https://maidan.world/problems/idempotency-key-reused",
            Self::UnprocessableEntity(_) => "https://maidan.world/problems/unprocessable-entity",
            Self::IdempotencyKeyInFlight => {
                "https://maidan.world/problems/idempotency-key-in-flight"
            }
            Self::CursorTooOld { .. } => "https://maidan.world/problems/cursor-too-old",
            Self::EventLogBroken { .. } => "https://maidan.world/problems/event-log-broken",
            Self::MediaQuotaExceeded { .. } => "https://maidan.world/problems/media-quota-exceeded",
            Self::FootprintConflict(_) => "https://maidan.world/problems/footprint-conflict",
            Self::PlanConflict(_) => "https://maidan.world/problems/plan-conflict",
            Self::ClaimLost(_) => "https://maidan.world/problems/claim-lost",
            Self::ThreadEnded(_) => "https://maidan.world/problems/thread-ended",
            Self::Gone(_) => "https://maidan.world/problems/gone",
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
    #[schema(example = "https://maidan.world/problems/not-found")]
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
    /// On a `media-quota-exceeded` 409: the bytes of media the workspace
    /// holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_bytes: Option<i64>,
    /// On a `media-quota-exceeded` 409: the workspace's media quota.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_bytes: Option<i64>,
    /// On a `media-quota-exceeded` 409: the size of the refused upload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_bytes: Option<i64>,
    /// On a `footprint-conflict` 409: the live claim items the footprint
    /// overlaps, each with its thread, holder and lease.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflicts: Option<Vec<maidan_types::FootprintConflict>>,
    /// On a `plan-conflict` 409: the children the plan already has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<maidan_types::PlannedChild>>,
    /// On a `conflict` 409 refusing values the workspace keeps unique among
    /// its open threads: the unique key set they were held under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_set: Option<Vec<maidan_types::UniqueAttributeKey>>,
    /// On a `conflict` 409 refusing values the workspace keeps unique among
    /// its open threads: the open thread that holds them, when the caller may
    /// read that thread.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holder_thread_id: Option<uuid::Uuid>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        // Inside the request span, so the line carries the request id the
        // client was given.
        if let Self::Internal(cause) = &self {
            tracing::error!(cause = %cause, "http.internal_error");
        }
        let snapshot = match &self {
            Self::CursorTooOld { snapshot, .. } => snapshot.clone(),
            _ => None,
        };
        let quota = match &self {
            Self::MediaQuotaExceeded {
                used_bytes,
                quota_bytes,
                upload_bytes,
            } => Some((*used_bytes, *quota_bytes, *upload_bytes)),
            _ => None,
        };
        let conflicts = match &self {
            Self::FootprintConflict(conflicts) => Some(conflicts.clone()),
            _ => None,
        };
        let children = match &self {
            Self::PlanConflict(children) => Some(children.clone()),
            _ => None,
        };
        let (key_set, holder_thread_id) = match &self {
            Self::AttributeHeld {
                key_set, holder, ..
            } => (Some(key_set.clone()), holder.map(|holder| holder.0)),
            _ => (None, None),
        };
        let body = ProblemDetails {
            type_: self.problem_type().to_string(),
            title: self.title().to_string(),
            status: status.as_u16(),
            detail: self.detail(),
            must_refetch: matches!(self, Self::CursorTooOld { .. }),
            snapshot,
            used_bytes: quota.map(|(used, _, _)| used),
            quota_bytes: quota.map(|(_, quota, _)| quota),
            upload_bytes: quota.map(|(_, _, upload)| upload),
            conflicts,
            children,
            key_set,
            holder_thread_id,
        };
        let mut response = (status, Json(body)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

impl From<maidan_store::media_retention::MediaQuotaExceeded> for ApiError {
    fn from(err: maidan_store::media_retention::MediaQuotaExceeded) -> Self {
        Self::MediaQuotaExceeded {
            used_bytes: err.used_bytes,
            quota_bytes: err.quota_bytes,
            upload_bytes: err.upload_bytes,
        }
    }
}

impl From<maidan_mcp::claim_lease::InvalidLease> for ApiError {
    fn from(err: maidan_mcp::claim_lease::InvalidLease) -> Self {
        Self::BadRequest(err.to_string())
    }
}

impl From<maidan_router::RouterError> for ApiError {
    fn from(err: maidan_router::RouterError) -> Self {
        match err {
            maidan_router::RouterError::Store(e) => e.into(),
        }
    }
}

impl From<maidan_store::change_pulls::ChangePullError> for ApiError {
    fn from(err: maidan_store::change_pulls::ChangePullError) -> Self {
        use maidan_store::change_pulls::ChangePullError;
        match err {
            ChangePullError::Invalid(msg) => Self::BadRequest(msg),
            ChangePullError::NotAllowed(msg) => Self::Forbidden(msg),
            ChangePullError::Store(err) => err.into(),
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
            StoreError::ClaimLost(msg) => Self::ClaimLost(msg),
            StoreError::Ended(msg) => Self::ThreadEnded(msg),
            StoreError::Gone(msg) => Self::Gone(msg),
            // Named only by a route that has checked the caller may read the
            // holder (`routes::disclose_holder`); by default it stays unnamed.
            StoreError::AttributeHeld { ref held, .. } => Self::AttributeHeld {
                key_set: held.iter().map(|(key, _)| *key).collect(),
                detail: err.to_string(),
                holder: None,
            },
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
            StoreError::ContentKey(e) => {
                tracing::error!(error = %e, "content key error");
                Self::Internal("content key error".into())
            }
            StoreError::OverAllowance(meter) => crate::usage_meters::cut_off_error(meter),
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
            "https://maidan.world/problems/cursor-too-old"
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
    fn an_ended_thread_is_409_and_a_gone_one_410_each_with_its_own_type() {
        let ended = ApiError::from(StoreError::Ended("archived".into()));
        assert_eq!(ended.status(), StatusCode::CONFLICT);
        assert_eq!(
            ended.problem_type(),
            "https://maidan.world/problems/thread-ended"
        );
        let gone = ApiError::from(StoreError::Gone("deleted".into()));
        assert_eq!(gone.status(), StatusCode::GONE);
        assert_eq!(gone.problem_type(), "https://maidan.world/problems/gone");
        assert_eq!(gone.into_response().status(), StatusCode::GONE);
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
            "https://maidan.world/problems/event-log-broken"
        );
        assert!(err.detail().contains("id=4"));
        assert!(err.detail().contains("content_hash_mismatch"));
        assert_eq!(err.into_response().status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn an_internal_error_answers_a_fixed_detail_and_never_its_cause() {
        let cause = "connect /var/lib/maidan/secret.sock: FEDERATION_ENCRYPTION_KEY unset";
        let err = ApiError::Internal(cause.into());
        assert_eq!(err.detail(), INTERNAL_DETAIL);
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let text = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(!text.contains("secret.sock"), "{text}");
        assert!(!text.contains("FEDERATION_ENCRYPTION_KEY"), "{text}");
        let problem: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(problem["type"], "https://maidan.world/problems/internal");
        assert_eq!(problem["detail"], INTERNAL_DETAIL);
    }
}
