//! W3C Trace Context on the way in and on the way out.
//!
//! Every HTTP request (REST, the WebSocket upgrade, MCP, A2A JSON-RPC) and
//! every A2A gRPC call accepts `traceparent`. The work runs as a child of
//! that span — or as a new root when the header is missing or not a
//! traceparent — and the response carries `traceresponse` naming the server
//! span. Outbound calls stamp that same span as their `traceparent`, so a
//! webhook or a projector stays in the caller's trace after the request
//! itself has finished.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};
use maidan_types::TraceContext;
use tower::{Layer, Service};
use tracing::Instrument;

const TRACEPARENT: &str = "traceparent";
const TRACESTATE: &str = "tracestate";
const TRACERESPONSE: &str = "traceresponse";

fn header_str(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// The server span for an incoming header pair: a child of a traceparent
/// that parses, otherwise a new root.
pub fn server_span(traceparent: Option<&str>, tracestate: Option<&str>) -> TraceContext {
    match traceparent.and_then(|header| TraceContext::parse(header, tracestate)) {
        Some(parent) => parent.child(),
        None => TraceContext::root(),
    }
}

fn span_for(headers: &axum::http::HeaderMap) -> (Option<TraceContext>, TraceContext) {
    let traceparent = header_str(headers, TRACEPARENT);
    let tracestate = header_str(headers, TRACESTATE);
    let incoming = traceparent
        .as_deref()
        .and_then(|header| TraceContext::parse(header, tracestate.as_deref()));
    let fallback = match &incoming {
        Some(parent) => parent.child(),
        None => TraceContext::root(),
    };
    (incoming, fallback)
}

/// Accept `traceparent` on every HTTP request and name the server span in
/// `traceresponse`.
pub async fn middleware(req: Request, next: Next) -> Response {
    let (incoming, fallback) = span_for(req.headers());
    let span = tracing::info_span!("inbound");
    if let Some(parent) = &incoming {
        maidan_observability::adopt_remote_parent(&span, parent);
    }
    let tracestate = fallback.tracestate().map(str::to_string);
    let (mut response, server) = async move {
        // Prefer the exported span's ids when they continued the same trace,
        // so `traceresponse` and the collector name one span.
        let recorded = maidan_observability::recorded_span(tracestate.clone())
            .filter(|recorded| recorded.trace_id() == fallback.trace_id());
        let server = recorded.unwrap_or(fallback);
        let response = maidan_store::trace::scope(server.clone(), next.run(req)).await;
        (response, server)
    }
    .instrument(span)
    .await;
    if let Ok(value) = HeaderValue::from_str(&server.traceparent()) {
        response
            .headers_mut()
            .insert(axum::http::HeaderName::from_static(TRACERESPONSE), value);
    }
    response
}

/// Add `traceparent` (and `tracestate`, when there is one) from the task's
/// current server span. A task with no trace is left unchanged.
pub fn stamp(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    let Some(trace) = maidan_store::trace::current() else {
        return builder;
    };
    let mut builder = builder.header(TRACEPARENT, trace.traceparent());
    if let Some(state) = trace.tracestate() {
        builder = builder.header(TRACESTATE, state);
    }
    builder
}

/// Tower layer that continues `traceparent` on the A2A gRPC server. gRPC
/// metadata is HTTP headers, so the same header names apply.
#[derive(Clone, Copy, Debug, Default)]
pub struct GrpcTraceLayer;

impl<S> Layer<S> for GrpcTraceLayer {
    type Service = GrpcTrace<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcTrace { inner }
    }
}

/// See [`GrpcTraceLayer`].
#[derive(Clone, Debug)]
pub struct GrpcTrace<S> {
    inner: S,
}

impl<S, B, ResBody> Service<axum::http::Request<B>> for GrpcTrace<S>
where
    S: Service<axum::http::Request<B>, Response = axum::http::Response<ResBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send,
    S::Error: Send,
    B: Send + 'static,
    ResBody: Send,
{
    type Response = axum::http::Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::http::Request<B>) -> Self::Future {
        let (incoming, fallback) = span_for(req.headers());
        let span = tracing::info_span!("inbound");
        if let Some(parent) = &incoming {
            maidan_observability::adopt_remote_parent(&span, parent);
        }
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let tracestate = fallback.tracestate().map(str::to_string);
        Box::pin(
            async move {
                let recorded = maidan_observability::recorded_span(tracestate)
                    .filter(|recorded| recorded.trace_id() == fallback.trace_id());
                let server = recorded.unwrap_or(fallback);
                let mut response =
                    maidan_store::trace::scope(server.clone(), inner.call(req)).await?;
                if let Ok(value) = axum::http::HeaderValue::from_str(&server.traceparent()) {
                    response
                        .headers_mut()
                        .insert(axum::http::HeaderName::from_static(TRACERESPONSE), value);
                }
                Ok(response)
            }
            .instrument(span),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::routing::get;
    use axum::{body::Body, Router};
    use maidan_types::TraceContext;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn an_incoming_traceparent_is_continued_on_the_response() {
        let app = Router::new()
            .route(
                "/x",
                get(|| async {
                    maidan_store::trace::current()
                        .map(|trace| trace.traceparent())
                        .unwrap_or_default()
                }),
            )
            .layer(axum::middleware::from_fn(middleware));
        let parent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/x")
                    .header(TRACEPARENT, parent)
                    .header(TRACESTATE, "vendor=one")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let traceresponse = response
            .headers()
            .get(TRACERESPONSE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let inside = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(inside, traceresponse);
        let continued = TraceContext::parse(&traceresponse, Some("vendor=one")).unwrap();
        let incoming = TraceContext::parse(parent, None).unwrap();
        assert_eq!(continued.trace_id(), incoming.trace_id());
        assert_ne!(continued.span_id(), incoming.span_id());
        assert_eq!(continued.tracestate(), Some("vendor=one"));
    }

    #[tokio::test]
    async fn a_bad_traceparent_starts_a_fresh_trace_instead_of_failing() {
        let app = Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(middleware));
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/x")
                    .header(TRACEPARENT, "not-a-trace")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let traceresponse = response
            .headers()
            .get(TRACERESPONSE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(TraceContext::parse(traceresponse, None).is_some());
    }

    #[tokio::test]
    async fn stamp_puts_the_task_trace_on_the_outbound_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let _ = sock
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await;
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
        });
        let trace = TraceContext::parse(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            Some("vendor=one"),
        )
        .unwrap();
        let client = reqwest::Client::new();
        maidan_store::trace::scope(trace.clone(), async {
            stamp(client.get(format!("http://{addr}/")))
                .send()
                .await
                .unwrap();
        })
        .await;
        let raw = rx.await.unwrap().to_ascii_lowercase();
        assert!(raw.contains(&trace.traceparent()));
        assert!(raw.contains("tracestate: vendor=one"));
    }
}
