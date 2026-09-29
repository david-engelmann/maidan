//! Fail-fast load shedding: a ceiling on in-flight HTTP requests.
//!
//! Past the ceiling a request is answered at once with a `503` problem and
//! `Retry-After: 1` instead of queueing behind the work already running. The
//! layer sits outside the rate limiter and authentication (see `app.rs`), so a
//! shed request costs no Redis round-trip, token lookup or pooled connection:
//! under overload the server refuses cheaply rather than slowing everyone
//! down until clients time out and retry into the pile.
//!
//! A request holds its permit until its response head is ready. A streaming
//! body (SSE, a download) and an upgraded WebSocket do not hold one; those have
//! their own ceilings. A long-poll such as an MCP wait tool holds one for as
//! long as it waits. Health probes and `/metrics` are never shed, so an
//! overloaded replica is not also restarted or blinded.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue},
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics::counter;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{error::ApiError, state::AppState};

/// Default in-flight request ceiling (`MAIDAN_MAX_CONCURRENT_REQUESTS`).
pub const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 1024;

/// The in-flight request ceiling. Cloning shares the permits.
#[derive(Clone, Debug)]
pub struct RequestLimit {
    permits: Option<Arc<Semaphore>>,
    max: usize,
}

/// Whether a request may run.
#[derive(Debug)]
pub enum Admission {
    /// Shedding is off.
    Unlimited,
    /// A permit, held until the response head is ready.
    Admitted(OwnedSemaphorePermit),
    /// Every permit is taken.
    Refused,
}

impl RequestLimit {
    /// A ceiling of `max` in-flight requests; `0` turns shedding off.
    pub fn new(max: usize) -> Self {
        Self {
            permits: (max > 0).then(|| Arc::new(Semaphore::new(max))),
            max,
        }
    }

    /// Read `MAIDAN_MAX_CONCURRENT_REQUESTS`.
    pub fn from_env() -> Self {
        Self::new(parse_max(
            std::env::var("MAIDAN_MAX_CONCURRENT_REQUESTS").ok(),
        ))
    }

    /// The configured ceiling; `0` when shedding is off.
    pub fn max(&self) -> usize {
        self.max
    }

    /// Take a permit if one is free; never waits.
    pub fn admit(&self) -> Admission {
        match &self.permits {
            None => Admission::Unlimited,
            Some(permits) => match permits.clone().try_acquire_owned() {
                Ok(permit) => Admission::Admitted(permit),
                Err(_) => Admission::Refused,
            },
        }
    }

    /// Requests holding a permit right now.
    pub fn in_flight(&self) -> usize {
        self.permits
            .as_ref()
            .map_or(0, |permits| self.max - permits.available_permits())
    }
}

impl Default for RequestLimit {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONCURRENT_REQUESTS)
    }
}

/// An unset or unparsable value keeps the default; `0` turns shedding off.
fn parse_max(raw: Option<String>) -> usize {
    raw.and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_MAX_CONCURRENT_REQUESTS)
}

/// Paths that are never shed: the same ones the rate limiter exempts.
pub(crate) fn exempt_path(path: &str) -> bool {
    crate::rate_limit::exempt_path(path)
}

/// The answer to a shed request.
pub fn overloaded() -> Response {
    let mut response = ApiError::Overloaded(
        "the server is at its in-flight request limit; retry after the Retry-After delay"
            .to_string(),
    )
    .into_response();
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

/// Shed a request when every permit is taken; otherwise hold one while the
/// rest of the stack runs.
pub async fn middleware(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if exempt_path(request.uri().path()) {
        return next.run(request).await;
    }
    match state.request_limit.admit() {
        Admission::Unlimited => next.run(request).await,
        Admission::Admitted(_permit) => next.run(request).await,
        Admission::Refused => {
            counter!("maidan_http_shed_total").increment(1);
            overloaded()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_or_garbled_ceiling_keeps_the_default_and_zero_turns_it_off() {
        assert_eq!(parse_max(None), DEFAULT_MAX_CONCURRENT_REQUESTS);
        assert_eq!(
            parse_max(Some("lots".into())),
            DEFAULT_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(
            parse_max(Some("-3".into())),
            DEFAULT_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(parse_max(Some(" 64 ".into())), 64);
        assert_eq!(parse_max(Some("0".into())), 0);
    }

    #[test]
    fn a_zero_ceiling_holds_no_permits() {
        let limit = RequestLimit::new(0);
        assert!(limit.permits.is_none());
        assert_eq!(limit.in_flight(), 0);
        assert_eq!(limit.max(), 0);
    }

    #[test]
    fn in_flight_counts_the_permits_taken_and_a_full_limit_refuses() {
        let limit = RequestLimit::new(2);
        let Admission::Admitted(first) = limit.admit() else {
            panic!("a free permit must be admitted");
        };
        let Admission::Admitted(_second) = limit.admit() else {
            panic!("a free permit must be admitted");
        };
        assert_eq!(limit.in_flight(), 2);
        assert!(matches!(limit.admit(), Admission::Refused));
        drop(first);
        assert_eq!(limit.in_flight(), 1);
        assert!(matches!(limit.admit(), Admission::Admitted(_)));
    }

    #[test]
    fn a_zero_ceiling_admits_everything() {
        assert!(matches!(RequestLimit::new(0).admit(), Admission::Unlimited));
    }

    #[test]
    fn health_probes_and_metrics_are_never_shed() {
        assert!(exempt_path("/health/live"));
        assert!(exempt_path("/health/ready"));
        assert!(exempt_path("/metrics"));
        assert!(!exempt_path("/workspaces"));
        assert!(!exempt_path("/mcp"));
    }

    #[tokio::test]
    async fn a_shed_request_is_a_503_problem_with_retry_after() {
        let response = overloaded();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(response.headers()[header::RETRY_AFTER], "1");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let problem: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(problem["type"], "https://maidan.dev/problems/overloaded");
        assert_eq!(problem["status"], 503);
    }
}
