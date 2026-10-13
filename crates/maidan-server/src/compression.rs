//! Gzip response compression for the HTTP surface.
//!
//! The big documents this server hands out — the OpenAPI document, the
//! MCP tool catalog inside JSON-RPC answers, board assets — are JSON and
//! text that shrink five-to-ten-fold, and until now they crossed the wire
//! unencoded. tower-http's layer negotiates from `Accept-Encoding` and
//! its default predicate already refuses what must not be encoded:
//! server-sent events (a compressed stream would hold small events back
//! in the compressor), gRPC frames, images, and bodies under 32 bytes;
//! the layer itself never encodes a response that already carries a
//! `Content-Encoding`.
//!
//! One refusal is this module's: a `101 Switching Protocols` response.
//! Its body is the upgraded connection (the WebSocket subscribe path),
//! not a document, and marking it `Content-Encoding: gzip` invites a
//! client to mishandle the upgrade.

use axum::http::{Response, StatusCode};
use http_body::Body;
use tower_http::compression::{
    predicate::{DefaultPredicate, Predicate},
    CompressionLayer,
};

/// The compression layer for the shared router stack, placed outside the
/// panic guard so a panic's 500 problem compresses like any other body,
/// and inside the metrics middleware so the recorded response is the one
/// that actually left the server.
pub fn layer() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new().compress_when(DefaultPredicate::new().and(NotAnUpgrade).and(NotNoStore))
}

/// Refuses compression for protocol upgrades: see the module docs.
#[derive(Debug, Clone, Copy)]
struct NotAnUpgrade;

impl Predicate for NotAnUpgrade {
    fn should_compress<B>(&self, response: &Response<B>) -> bool
    where
        B: Body,
    {
        response.status() != StatusCode::SWITCHING_PROTOCOLS
    }
}

/// Refuses compression for a response marked `Cache-Control: no-store`: the
/// routes that answer with a credential set it, and compressing a secret beside
/// text the caller chose turns its length into an oracle (BREACH).
#[derive(Debug, Clone, Copy)]
struct NotNoStore;

impl Predicate for NotNoStore {
    fn should_compress<B>(&self, response: &Response<B>) -> bool
    where
        B: Body,
    {
        !response
            .headers()
            .get_all(axum::http::header::CACHE_CONTROL)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .any(|v| {
                v.split(',')
                    .any(|d| d.trim().eq_ignore_ascii_case("no-store"))
            })
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body as AxumBody;
    use axum::http::header::CONTENT_TYPE;

    use super::*;

    fn response(
        status: StatusCode,
        content_type: Option<&str>,
        body: &'static str,
    ) -> Response<AxumBody> {
        let mut builder = Response::builder().status(status);
        if let Some(content_type) = content_type {
            builder = builder.header(CONTENT_TYPE, content_type);
        }
        builder.body(AxumBody::from(body)).expect("response builds")
    }

    #[test]
    fn a_json_document_compresses() {
        let predicate = DefaultPredicate::new().and(NotAnUpgrade);
        let res = response(
            StatusCode::OK,
            Some("application/json"),
            "{\"tools\": [\"a long enough document to be worth encoding\"]}",
        );
        assert!(predicate.should_compress(&res));
    }

    #[test]
    fn an_event_stream_never_compresses() {
        let predicate = DefaultPredicate::new().and(NotAnUpgrade);
        let res = response(
            StatusCode::OK,
            Some("text/event-stream"),
            "data: {\"kind\": \"message_posted\"}\n\n",
        );
        assert!(!predicate.should_compress(&res));
    }

    #[test]
    fn a_no_store_response_never_compresses() {
        let predicate = DefaultPredicate::new().and(NotAnUpgrade).and(NotNoStore);
        let mut res = response(
            StatusCode::CREATED,
            Some("application/json"),
            "{\"secret\": \"a long enough credential body to be worth encoding\"}",
        );
        res.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("private, no-store"),
        );
        assert!(!predicate.should_compress(&res));
    }

    #[test]
    fn a_protocol_upgrade_never_compresses() {
        let predicate = DefaultPredicate::new().and(NotAnUpgrade);
        let res = response(
            StatusCode::SWITCHING_PROTOCOLS,
            Some("application/json"),
            "{\"tools\": [\"a long enough document to be worth encoding\"]}",
        );
        assert!(!predicate.should_compress(&res));
    }
}
