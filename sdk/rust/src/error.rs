//! Errors: a variant per RFC 9457 problem `type` the server documents.

use std::fmt;

use serde_json::Value;

/// The URI prefix of every problem type the server emits.
pub const PROBLEM_BASE: &str = "https://maidan.dev/problems/";

/// The problem a failed response carried.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Problem {
    pub status: u16,
    /// The problem `type` URI; `None` when the body was not a problem.
    pub problem_type: Option<String>,
    pub title: Option<String>,
    /// The problem's `detail`, or the body's text when it was not a problem.
    pub detail: Option<String>,
    /// The problem body as sent, unknown members included.
    pub raw: Option<Value>,
    /// Seconds from `Retry-After` (sent on 429 and 503).
    pub retry_after: Option<f64>,
}

impl Problem {
    /// On a cursor-too-old 409: the snapshot path covering the pruned prefix.
    pub fn snapshot(&self) -> Option<&str> {
        self.raw.as_ref()?.get("snapshot")?.as_str()
    }
}

/// A failed call. Each problem type the server documents is its own variant;
/// [`MaidanError::Unknown`] is a type this crate does not know, or a body that
/// is not a problem.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum MaidanError {
    /// 404 `not-found`.
    NotFound(Box<Problem>),
    /// 405 `method-not-allowed`.
    MethodNotAllowed(Box<Problem>),
    /// 409 `conflict`: the resource's state refuses the change.
    Conflict(Box<Problem>),
    /// 400 `bad-request`.
    BadRequest(Box<Problem>),
    /// 401 `unauthorized`: missing or invalid bearer token.
    Unauthorized(Box<Problem>),
    /// 401 `invalid-signature` (webhook ingress).
    InvalidSignature(Box<Problem>),
    /// 403 `forbidden`: a missing capability or channel access. Not retryable.
    Forbidden(Box<Problem>),
    /// 413 `payload-too-large`.
    PayloadTooLarge(Box<Problem>),
    /// 415 `unsupported-media-type`.
    UnsupportedMediaType(Box<Problem>),
    /// 429 `rate-limited`; see [`Problem::retry_after`].
    RateLimited(Box<Problem>),
    /// 502 `bad-gateway`.
    BadGateway(Box<Problem>),
    /// 500 `internal`.
    Internal(Box<Problem>),
    /// 503 `overloaded`: refused without running; retry after `retry_after`.
    Overloaded(Box<Problem>),
    /// 422 `idempotency-key-reused`: the key was used for a different request.
    IdempotencyKeyReused(Box<Problem>),
    /// 409 `idempotency-key-in-flight`: the first request with the key still runs.
    IdempotencyKeyInFlight(Box<Problem>),
    /// 409 `cursor-too-old`: refetch from [`Problem::snapshot`], never clamp.
    CursorTooOld(Box<Problem>),
    /// 409 `event-log-broken`: the hash chain failed verification.
    EventLogBroken(Box<Problem>),
    /// A problem type this crate does not know, or a body that is not a problem.
    Unknown(Box<Problem>),
    /// No HTTP answer: connect, TLS, socket or WebSocket failure.
    Transport(String),
    /// A 2xx body that did not decode into its model.
    Decode(String),
}

/// The problem types the server documents, by the last segment of their URI.
pub const PROBLEM_TYPES: [&str; 17] = [
    "not-found",
    "method-not-allowed",
    "conflict",
    "bad-request",
    "unauthorized",
    "invalid-signature",
    "forbidden",
    "payload-too-large",
    "unsupported-media-type",
    "rate-limited",
    "bad-gateway",
    "internal",
    "overloaded",
    "idempotency-key-reused",
    "idempotency-key-in-flight",
    "cursor-too-old",
    "event-log-broken",
];

impl MaidanError {
    pub(crate) fn transport(msg: impl Into<String>) -> Self {
        Self::Transport(msg.into())
    }

    /// The error for a failed response: the variant its problem `type` names.
    pub fn from_response(status: u16, body: &[u8], retry_after: Option<f64>) -> Self {
        let raw = serde_json::from_slice::<Value>(body)
            .ok()
            .filter(Value::is_object);
        let field = |k: &str| {
            raw.as_ref()
                .and_then(|v| v.get(k))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let problem_type = field("type");
        let detail = if raw.is_some() {
            field("detail")
        } else {
            Some(String::from_utf8_lossy(body).into_owned()).filter(|s| !s.is_empty())
        };
        let problem = Box::new(Problem {
            status,
            title: field("title"),
            detail,
            problem_type: problem_type.clone(),
            raw,
            retry_after,
        });
        let segment = problem_type
            .as_deref()
            .and_then(|t| t.strip_prefix(PROBLEM_BASE));
        match segment {
            Some("not-found") => Self::NotFound(problem),
            Some("method-not-allowed") => Self::MethodNotAllowed(problem),
            Some("conflict") => Self::Conflict(problem),
            Some("bad-request") => Self::BadRequest(problem),
            Some("unauthorized") => Self::Unauthorized(problem),
            Some("invalid-signature") => Self::InvalidSignature(problem),
            Some("forbidden") => Self::Forbidden(problem),
            Some("payload-too-large") => Self::PayloadTooLarge(problem),
            Some("unsupported-media-type") => Self::UnsupportedMediaType(problem),
            Some("rate-limited") => Self::RateLimited(problem),
            Some("bad-gateway") => Self::BadGateway(problem),
            Some("internal") => Self::Internal(problem),
            Some("overloaded") => Self::Overloaded(problem),
            Some("idempotency-key-reused") => Self::IdempotencyKeyReused(problem),
            Some("idempotency-key-in-flight") => Self::IdempotencyKeyInFlight(problem),
            Some("cursor-too-old") => Self::CursorTooOld(problem),
            Some("event-log-broken") => Self::EventLogBroken(problem),
            _ => Self::Unknown(problem),
        }
    }

    /// The problem this error carries; `None` for transport and decode errors.
    pub fn problem(&self) -> Option<&Problem> {
        match self {
            Self::NotFound(p)
            | Self::MethodNotAllowed(p)
            | Self::Conflict(p)
            | Self::BadRequest(p)
            | Self::Unauthorized(p)
            | Self::InvalidSignature(p)
            | Self::Forbidden(p)
            | Self::PayloadTooLarge(p)
            | Self::UnsupportedMediaType(p)
            | Self::RateLimited(p)
            | Self::BadGateway(p)
            | Self::Internal(p)
            | Self::Overloaded(p)
            | Self::IdempotencyKeyReused(p)
            | Self::IdempotencyKeyInFlight(p)
            | Self::CursorTooOld(p)
            | Self::EventLogBroken(p)
            | Self::Unknown(p) => Some(p),
            Self::Transport(_) | Self::Decode(_) => None,
        }
    }

    /// The HTTP status; 0 when there was no HTTP answer.
    pub fn status(&self) -> u16 {
        self.problem().map_or(0, |p| p.status)
    }

    /// Seconds from `Retry-After`, when the server sent one.
    pub fn retry_after(&self) -> Option<f64> {
        self.problem().and_then(|p| p.retry_after)
    }

    /// A 409.
    pub fn is_conflict(&self) -> bool {
        self.status() == 409
    }
    /// A 409 `must_refetch` / `cursor-too-old` — fail loud, never clamp the cursor.
    pub fn is_cursor_too_old(&self) -> bool {
        match self.problem() {
            Some(p) if p.status == 409 => {
                matches!(self, Self::CursorTooOld(_))
                    || p.raw
                        .as_ref()
                        .and_then(|v| v.get("must_refetch"))
                        .and_then(Value::as_bool)
                        == Some(true)
                    || p.problem_type.as_deref() == Some("cursor_too_old")
            }
            _ => false,
        }
    }
    /// A 403 (missing capability / channel access — not retryable).
    pub fn is_forbidden(&self) -> bool {
        self.status() == 403
    }
    /// A 429 (server rate limit).
    pub fn is_rate_limited(&self) -> bool {
        self.status() == 429
    }
    /// No HTTP answer.
    pub fn is_transport(&self) -> bool {
        matches!(self, Self::Transport(_))
    }
}

impl fmt::Display for MaidanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(msg) => write!(f, "maidan: transport: {msg}"),
            Self::Decode(msg) => write!(f, "maidan: decoding a response: {msg}"),
            _ => match self.problem() {
                Some(Problem {
                    status,
                    detail: Some(detail),
                    ..
                }) => write!(f, "maidan: request failed: HTTP {status}: {detail}"),
                Some(p) => write!(f, "maidan: request failed: HTTP {}", p.status),
                None => Ok(()),
            },
        }
    }
}

impl std::error::Error for MaidanError {}
