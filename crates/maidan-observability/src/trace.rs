//! Link a `tracing` span to a W3C trace carried in from a caller.
//!
//! Without the `otel` feature these are no-ops: the header is still stored
//! and sent, it just is not a parent in an exported trace.

use maidan_types::TraceContext;

/// Make `span` a child of `parent` before the span is entered. After the
/// span has been built this cannot change its parent; call it first.
pub fn adopt_remote_parent(span: &tracing::Span, parent: &TraceContext) {
    #[cfg(feature = "otel")]
    {
        use opentelemetry::trace::{
            SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
        };
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        let remote = SpanContext::new(
            TraceId::from_bytes(*parent.trace_id()),
            SpanId::from_bytes(*parent.span_id()),
            if parent.sampled() {
                TraceFlags::SAMPLED
            } else {
                TraceFlags::default()
            },
            true,
            TraceState::default(),
        );
        let context = opentelemetry::Context::new().with_remote_span_context(remote);
        let _ = span.set_parent(context);
    }
    #[cfg(not(feature = "otel"))]
    {
        let _ = (span, parent);
    }
}

/// Ids of the span currently entered, once the OpenTelemetry layer has built
/// it. `None` when that layer is not installed or the span is not a local one.
pub fn recorded_span(tracestate: Option<String>) -> Option<TraceContext> {
    #[cfg(feature = "otel")]
    {
        use opentelemetry::trace::TraceContextExt;
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        let context = tracing::Span::current().context();
        let span_context = context.span().span_context().clone();
        if !span_context.is_valid() || span_context.is_remote() {
            return None;
        }
        TraceContext::from_ids(
            span_context.trace_id().to_bytes(),
            span_context.span_id().to_bytes(),
            span_context.trace_flags().is_sampled(),
            tracestate,
        )
    }
    #[cfg(not(feature = "otel"))]
    {
        let _ = tracestate;
        None
    }
}
