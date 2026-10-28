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
    // The route template, never the path: ids in the path stay out of a
    // field a dashboard groups by. The thread a request names is its own
    // field, so a caller's per-thread trace can find the requests it made.
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|matched| matched.as_str().to_string());
    let thread = route
        .as_deref()
        .and_then(|route| thread_in_path(route, req.uri().path()));
    let span = tracing::info_span!(
        "inbound",
        http.route = route.as_deref().unwrap_or("-"),
        http.request.method = %req.method(),
        maidan.thread_id = tracing::field::Empty,
    );
    if let Some(thread) = thread {
        span.record("maidan.thread_id", tracing::field::display(thread));
    }
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

/// The thread id in `path` where its route template names one
/// (`/threads/{id}/…`, `/ui/api/threads/{tid}`), as a UUID.
fn thread_in_path(route: &str, path: &str) -> Option<uuid::Uuid> {
    let mut previous = "";
    for (template, actual) in route.split('/').zip(path.split('/')) {
        if previous == "threads" && template.starts_with('{') {
            return uuid::Uuid::parse_str(actual).ok();
        }
        previous = template;
    }
    None
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
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::{Context, SubscriberExt};

    /// Every field recorded on an `inbound` span, as `name=value`.
    #[derive(Clone, Default)]
    struct InboundFields(Arc<Mutex<Vec<String>>>);

    struct Visit<'a>(&'a mut Vec<String>);

    impl tracing::field::Visit for Visit<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push(format!("{}={:?}", field.name(), value));
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.push(format!("{}={value}", field.name()));
        }
    }

    impl<S> tracing_subscriber::Layer<S> for InboundFields
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _: &tracing::span::Id,
            _: Context<'_, S>,
        ) {
            if attrs.metadata().name() == "inbound"
                && let Ok(mut fields) = self.0.lock()
            {
                attrs.record(&mut Visit(&mut fields));
            }
        }
        fn on_record(
            &self,
            id: &tracing::span::Id,
            values: &tracing::span::Record<'_>,
            ctx: Context<'_, S>,
        ) {
            if ctx.span(id).is_some_and(|span| span.name() == "inbound")
                && let Ok(mut fields) = self.0.lock()
            {
                values.record(&mut Visit(&mut fields));
            }
        }
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods)] // a bare route, not the API
    async fn the_inbound_span_names_the_route_template_and_the_thread() {
        let fields = InboundFields::default();
        let subscriber = tracing_subscriber::registry().with(fields.clone());
        let _guard = tracing::subscriber::set_default(subscriber);
        let app = axum::Router::new()
            .route(
                "/threads/{id}/result",
                axum::routing::get(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn(super::middleware));
        let id = "01a17dc9-bb2f-73b1-87ad-41c77759c767";
        let response = app
            .oneshot(
                axum::http::Request::get(format!("/threads/{id}/result"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        let fields = fields.0.lock().unwrap().clone();
        assert!(
            fields.contains(&"http.route=/threads/{id}/result".to_string()),
            "{fields:?}"
        );
        assert!(
            fields.contains(&"http.request.method=GET".to_string()),
            "{fields:?}"
        );
        assert!(
            fields.contains(&format!("maidan.thread_id={id}")),
            "{fields:?}"
        );
        // The template, not the path: no id reaches the route field.
        assert!(!fields
            .iter()
            .any(|f| f.starts_with("http.route=") && f.contains(id)));
    }

    #[test]
    fn the_thread_id_comes_from_a_threads_route_and_nothing_else() {
        let id = "01a17dc9-bb2f-73b1-87ad-41c77759c767";
        let thread = uuid::Uuid::parse_str(id).ok();
        assert_eq!(
            super::thread_in_path("/threads/{id}/result", &format!("/threads/{id}/result")),
            thread
        );
        assert_eq!(
            super::thread_in_path("/ui/api/threads/{tid}", &format!("/ui/api/threads/{id}")),
            thread
        );
        assert_eq!(
            super::thread_in_path(
                "/workspaces/{id}/threads",
                &format!("/workspaces/{id}/threads")
            ),
            None
        );
        assert_eq!(
            super::thread_in_path("/threads/{id}", "/threads/not-a-uuid"),
            None
        );
    }

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
