//! A byte budget for artifact uploads held in memory.
//!
//! A single-shot upload is buffered whole before it is hashed, and the
//! request ceiling (`MAIDAN_MAX_CONCURRENT_REQUESTS`) counts requests, not
//! bytes: a thousand admitted uploads at the artifact limit would hold a
//! thousand times that limit. Each upload is charged its `Content-Length`
//! (the whole artifact limit when the length is not declared, since a chunked
//! body can grow to it) against `MAIDAN_ARTIFACT_UPLOAD_BUDGET_BYTES`, and an
//! upload that does not fit is refused at once with a 503 and `Retry-After`,
//! like a shed request, instead of queueing while it holds a connection.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue},
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics::counter;
use tokio::sync::Semaphore;

use crate::error::ApiError;

/// Default budget: 256 MiB of upload bodies in memory at once.
const DEFAULT_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Permits are KiB, so a budget of several GiB still fits the semaphore's
/// `u32` acquire count.
const UNIT: usize = 1024;

#[derive(Debug, Clone)]
pub struct UploadBudget {
    permits: Option<Arc<Semaphore>>,
    /// The most one upload can be charged: the artifact body limit.
    max_upload: usize,
}

impl UploadBudget {
    /// A budget of `budget` bytes, `0` for none. It is raised to at least one
    /// upload at `max_upload`, or the largest allowed upload could never run.
    pub fn new(budget: usize, max_upload: usize) -> Self {
        let permits = (budget > 0).then(|| {
            let bytes = budget.max(max_upload);
            Arc::new(Semaphore::new(units(bytes) as usize))
        });
        Self {
            permits,
            max_upload,
        }
    }

    /// Read `MAIDAN_ARTIFACT_UPLOAD_BUDGET_BYTES`, against the artifact body
    /// limit the upload routes enforce.
    pub fn from_env(max_upload: usize) -> Self {
        Self::new(
            parse_budget(std::env::var("MAIDAN_ARTIFACT_UPLOAD_BUDGET_BYTES").ok()),
            max_upload,
        )
    }

    /// What an upload declaring `content_length` is charged, or `None` when
    /// it declares more than the limit: that upload is refused with 413 by the
    /// body limit whatever the budget holds, so it must not be answered with
    /// a retryable 503 first.
    fn charge(&self, content_length: Option<usize>) -> Option<u32> {
        match content_length {
            Some(len) if len > self.max_upload => None,
            Some(len) => Some(units(len)),
            None => Some(units(self.max_upload)),
        }
    }
}

fn units(bytes: usize) -> u32 {
    u32::try_from(bytes.div_ceil(UNIT).max(1)).unwrap_or(u32::MAX)
}

fn parse_budget(raw: Option<String>) -> usize {
    match raw.as_deref().map(str::trim) {
        None | Some("") => DEFAULT_BUDGET_BYTES,
        Some(value) => value.parse::<usize>().unwrap_or(DEFAULT_BUDGET_BYTES),
    }
}

fn budget_full() -> Response {
    let mut response = ApiError::Overloaded(
        "artifact uploads in memory are at their byte budget; retry after the Retry-After delay"
            .to_string(),
    )
    .into_response();
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

/// Charge an upload against the budget for as long as its handler runs.
pub async fn middleware(
    State(budget): State<UploadBudget>,
    request: Request,
    next: Next,
) -> Response {
    let Some(permits) = budget.permits.clone() else {
        return next.run(request).await;
    };
    let content_length = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    let Some(charge) = budget.charge(content_length) else {
        return next.run(request).await;
    };
    match permits.try_acquire_many_owned(charge) {
        Ok(_held) => next.run(request).await,
        Err(_) => {
            counter!("maidan_artifact_upload_budget_refused_total").increment(1);
            budget_full()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: usize = 1024 * 1024;

    #[test]
    fn an_unset_or_garbled_budget_keeps_the_default_and_zero_turns_it_off() {
        assert_eq!(parse_budget(None), DEFAULT_BUDGET_BYTES);
        assert_eq!(parse_budget(Some(" ".into())), DEFAULT_BUDGET_BYTES);
        assert_eq!(parse_budget(Some("lots".into())), DEFAULT_BUDGET_BYTES);
        assert_eq!(parse_budget(Some("0".into())), 0);
        assert!(UploadBudget::new(0, MIB).permits.is_none());
    }

    #[test]
    fn the_budget_always_fits_one_upload_at_the_limit() {
        let budget = UploadBudget::new(MIB, 4 * MIB);
        let permits = budget.permits.expect("a budget");
        assert_eq!(permits.available_permits(), units(4 * MIB) as usize);
    }

    #[test]
    fn an_undeclared_length_is_charged_the_whole_limit_and_an_oversized_one_nothing() {
        let budget = UploadBudget::new(8 * MIB, 4 * MIB);
        assert_eq!(budget.charge(None), Some(units(4 * MIB)));
        assert_eq!(budget.charge(Some(3000)), Some(3));
        assert_eq!(budget.charge(Some(0)), Some(1));
        assert_eq!(
            budget.charge(Some(10 * MIB)),
            None,
            "an upload over the limit goes to the body limit's 413, not the budget's 503"
        );
    }

    #[tokio::test]
    async fn an_upload_that_does_not_fit_beside_those_in_flight_is_refused() {
        let budget = UploadBudget::new(4 * MIB, 4 * MIB);
        let permits = budget.permits.clone().expect("a budget");
        let held = permits
            .clone()
            .try_acquire_many_owned(budget.charge(Some(3 * MIB)).expect("in limit"))
            .expect("the first fits");
        assert!(permits
            .clone()
            .try_acquire_many_owned(budget.charge(Some(2 * MIB)).expect("in limit"))
            .is_err());
        assert!(permits
            .clone()
            .try_acquire_many_owned(budget.charge(Some(MIB)).expect("in limit"))
            .is_ok());
        drop(held);
        assert!(permits
            .try_acquire_many_owned(budget.charge(None).expect("undeclared"))
            .is_ok());
    }

    #[tokio::test]
    async fn a_refusal_is_a_503_problem_with_retry_after() {
        let response = budget_full();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            response.headers().get(header::RETRY_AFTER),
            Some(&HeaderValue::from_static("1"))
        );
    }
}
