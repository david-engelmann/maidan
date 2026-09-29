//! A handler that panics answers with a `500` problem.
//!
//! Without this, a panic unwinds out of the connection task and hyper drops
//! the connection: the client sees a reset with no status and no request id,
//! and retries blind. `CatchPanicLayer` turns it into the same RFC 9457 body
//! as every other error. The panic message goes to the log under the request's
//! span (so it carries the `X-Request-Id` the client got back) and to
//! `maidan_http_panics_total`; the client never sees it, since it can name
//! internals.

use std::any::Any;

use axum::response::{IntoResponse, Response};
use metrics::counter;
use tower_http::catch_panic::CatchPanicLayer;

use crate::error::ApiError;

/// What a client is told when its request panicked.
pub const PANIC_DETAIL: &str =
    "the server failed while handling this request; the X-Request-Id response header names it in the server log";

/// The panic payload as text, when it is text.
fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic with a non-text payload")
}

/// Log and count a panic, and answer it.
pub fn response_for_panic(payload: Box<dyn Any + Send + 'static>) -> Response {
    tracing::error!(
        panic = panic_message(payload.as_ref()),
        "http.handler_panicked"
    );
    counter!("maidan_http_panics_total").increment(1);
    ApiError::Internal(PANIC_DETAIL.to_string()).into_response()
}

/// The layer `app.rs` installs around every route.
pub fn layer() -> CatchPanicLayer<fn(Box<dyn Any + Send + 'static>) -> Response> {
    CatchPanicLayer::custom(response_for_panic as fn(_) -> _)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_is_read_from_str_and_string_payloads() {
        let from_str: Box<dyn Any + Send> = Box::new("boom");
        assert_eq!(panic_message(from_str.as_ref()), "boom");
        let from_string: Box<dyn Any + Send> = Box::new(String::from("bang"));
        assert_eq!(panic_message(from_string.as_ref()), "bang");
        let opaque: Box<dyn Any + Send> = Box::new(7_u8);
        assert_eq!(
            panic_message(opaque.as_ref()),
            "a panic with a non-text payload"
        );
    }

    #[tokio::test]
    async fn a_panic_is_a_500_problem_that_does_not_repeat_the_message() {
        let response = response_for_panic(Box::new("secret internal detail"));
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let text = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(!text.contains("secret internal detail"), "{text}");
        let problem: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(problem["type"], "https://maidan.dev/problems/internal");
        assert_eq!(problem["detail"], PANIC_DETAIL);
    }
}
