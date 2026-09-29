//! Request tracing that cannot leak a credential.
//!
//! Every request gets a `request` span carrying its method, path and headers.
//! Before the span is made, the headers that carry a credential are marked
//! sensitive, and `HeaderValue`'s `Debug` prints a sensitive value as
//! `Sensitive`, so the span (and anything exported from it over OTLP) shows
//! that an `Authorization` header was sent but never what it said. The query
//! string is left out of the span, because an OAuth or OIDC redirect carries
//! its one-time `code` there.
//!
//! Response headers are marked the same way (`Set-Cookie` holds the browser
//! session, `Mcp-Session-Id` names a live MCP session) before the response is
//! logged.

use std::sync::Arc;

use axum::http::{header, HeaderName, Request};
use tower_http::{
    sensitive_headers::{SetSensitiveRequestHeadersLayer, SetSensitiveResponseHeadersLayer},
    trace::{DefaultOnResponse, MakeSpan, TraceLayer},
};
use tracing::{Level, Span};

/// Request headers whose value is a credential or a signature over one.
pub fn sensitive_request_headers() -> Vec<HeaderName> {
    vec![
        header::AUTHORIZATION,
        header::PROXY_AUTHORIZATION,
        header::COOKIE,
        HeaderName::from_static("mcp-session-id"),
        HeaderName::from_static("x-hub-signature-256"),
        HeaderName::from_static("x-slack-signature"),
    ]
}

/// Response headers whose value is a credential.
pub fn sensitive_response_headers() -> Vec<HeaderName> {
    vec![
        header::SET_COOKIE,
        HeaderName::from_static("mcp-session-id"),
    ]
}

/// Marks the request headers above sensitive. Must wrap the trace layer.
pub fn request_layer() -> SetSensitiveRequestHeadersLayer {
    SetSensitiveRequestHeadersLayer::from_shared(Arc::from(sensitive_request_headers()))
}

/// Marks the response headers above sensitive. Must sit inside the trace
/// layer, so the response is marked before it is logged.
pub fn response_layer() -> SetSensitiveResponseHeadersLayer {
    SetSensitiveResponseHeadersLayer::from_shared(Arc::from(sensitive_response_headers()))
}

/// The span every request runs in: method, path (no query) and headers.
#[derive(Clone, Copy, Debug, Default)]
pub struct RequestSpan;

impl<B> MakeSpan<B> for RequestSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        tracing::debug_span!(
            "request",
            method = %request.method(),
            path = %request.uri().path(),
            version = ?request.version(),
            headers = ?request.headers(),
        )
    }
}

/// The trace layer: [`RequestSpan`], with the response's status, latency and
/// headers logged at `DEBUG`.
pub fn trace_layer() -> TraceLayer<
    tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
    RequestSpan,
    tower_http::trace::DefaultOnRequest,
    DefaultOnResponse,
> {
    TraceLayer::new_for_http()
        .make_span_with(RequestSpan)
        .on_response(
            DefaultOnResponse::new()
                .level(Level::DEBUG)
                .include_headers(true),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_header_prints_as_sensitive() {
        let mut value = header::HeaderValue::from_static("Bearer mdn_secret");
        value.set_sensitive(true);
        assert_eq!(format!("{value:?}"), "Sensitive");
    }

    #[test]
    fn every_credential_header_is_listed() {
        let request = sensitive_request_headers();
        for name in [
            "authorization",
            "proxy-authorization",
            "cookie",
            "mcp-session-id",
            "x-hub-signature-256",
            "x-slack-signature",
        ] {
            assert!(
                request.iter().any(|h| h == name),
                "{name} is not marked sensitive on requests"
            );
        }
        let response = sensitive_response_headers();
        for name in ["set-cookie", "mcp-session-id"] {
            assert!(
                response.iter().any(|h| h == name),
                "{name} is not marked sensitive on responses"
            );
        }
    }
}
