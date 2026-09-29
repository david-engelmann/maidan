//! The trace a request is working inside, carried across `.await` on the
//! same task. A spawned task does not inherit it; capture [`current`] and
//! re-enter with [`scope`] at the spawn.

use std::future::Future;

use maidan_types::TraceContext;

tokio::task_local! {
    static ACTIVE: TraceContext;
}

/// The server span for this task, if a request or a relayed event set one.
pub fn current() -> Option<TraceContext> {
    ACTIVE.try_with(|trace| trace.clone()).ok()
}

/// Run `fut` with `trace` as [`current`].
pub async fn scope<F>(trace: TraceContext, fut: F) -> F::Output
where
    F: Future,
{
    ACTIVE.scope(trace, fut).await
}

/// [`scope`] when a trace was carried, otherwise `fut` unchanged.
pub async fn maybe_scope<F>(trace: Option<TraceContext>, fut: F) -> F::Output
where
    F: Future,
{
    match trace {
        Some(trace) => scope(trace, fut).await,
        None => fut.await,
    }
}

/// `(traceparent, tracestate)` to store on a row written by this task.
pub fn current_columns() -> (Option<String>, Option<String>) {
    match current() {
        Some(trace) => (
            Some(trace.traceparent()),
            trace.tracestate().map(str::to_string),
        ),
        None => (None, None),
    }
}
