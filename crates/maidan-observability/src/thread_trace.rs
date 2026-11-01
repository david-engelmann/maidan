//! One trace per thread for the agent swarm.
//!
//! A thread's trace id is its UUID, so every stage of its work (claim,
//! acknowledge, result, review verdict, delivery) lands in one trace without
//! storing anything on the thread. A seat's harness joins the same trace by
//! sending [`thread_trace`]'s `traceparent` with its own spans.
//!
//! Redaction contract: a stage span carries only the keys in
//! [`SPAN_ATTRIBUTE_KEYS`]. Ids (thread, workspace, member) are span
//! attributes and never metric labels. Free text an event carries (a
//! selector, an error, a note, a repo) is never recorded.

use maidan_types::{Event, ThreadId, TraceContext, WorkspaceId};
use tracing::field::Empty;

/// Every attribute key a thread stage span may carry. The tests hold the
/// spans to this list.
pub const SPAN_ATTRIBUTE_KEYS: &[&str] = &[
    "stage",
    "thread_id",
    "workspace_id",
    "member_id",
    "verdict",
    "surface",
    "outcome",
];

/// Delivery surfaces a span may name; anything else reads as `other`.
const SURFACES: &[&str] = &["github", "github_branch", "slack", "linear"];

/// Delivery outcomes a span may name; anything else reads as `other`.
const OUTCOMES: &[&str] = &["delivered", "failed", "skipped"];

/// The trace context every span of `thread_id` hangs under: the trace id is
/// the thread's UUID, and the parent span id is taken from its last eight
/// bytes (or `1` in the impossible case they are all zero).
pub fn thread_trace(thread_id: ThreadId) -> TraceContext {
    let trace_id = *thread_id.0.as_bytes();
    let mut span_id = [0u8; 8];
    span_id.copy_from_slice(&trace_id[8..]);
    if span_id == [0; 8] {
        span_id[7] = 1;
    }
    let trace_id = if trace_id == [0; 16] {
        let mut id = [0u8; 16];
        id[15] = 1;
        id
    } else {
        trace_id
    };
    TraceContext::from_ids(trace_id, span_id, true, None).unwrap_or_else(TraceContext::root)
}

/// The stage of a thread's work an event marks, or `None` for an event that
/// is not one.
fn stage(event: &Event) -> Option<&'static str> {
    Some(match event {
        Event::ThreadAssignmentChanged {
            assignee_id: Some(_),
            ..
        } => "claim",
        Event::ThreadAssignmentChanged {
            assignee_id: None, ..
        } => "release",
        Event::ClaimExpired { .. } => "claim_expired",
        Event::ClaimUnacknowledged { .. } => "claim_unacknowledged",
        Event::ClaimStalled { .. } => "claim_stalled",
        Event::ClaimFailed { .. } => "claim_failed",
        Event::ThreadResultSet { .. } => "result",
        Event::ReviewSubmitted { .. } => "review",
        Event::ResultDeliveryFailed { .. } => "delivery",
        Event::ThreadLanded { .. } => "landed",
        _ => return None,
    })
}

fn closed(set: &[&'static str], value: &str) -> &'static str {
    set.iter()
        .copied()
        .find(|known| *known == value)
        .unwrap_or("other")
}

fn stage_span(stage: &'static str, thread_id: ThreadId) -> tracing::Span {
    let span = tracing::info_span!(
        target: "maidan::thread",
        "maidan.thread",
        stage,
        thread_id = %thread_id,
        workspace_id = Empty,
        member_id = Empty,
        verdict = Empty,
        surface = Empty,
        outcome = Empty,
    );
    crate::adopt_remote_parent(&span, &thread_trace(thread_id));
    span
}

/// Record the stage span an event marks, under its thread's trace. Events
/// that mark no stage record nothing.
pub fn observe_event(event: &Event) {
    let (Some(stage), Some(thread_id)) = (stage(event), event.thread_id()) else {
        return;
    };
    let span = stage_span(stage, thread_id);
    if let Some(workspace_id) = event.workspace_id() {
        span.record("workspace_id", tracing::field::display(workspace_id));
    }
    if let Some(member_id) = event.member_id() {
        span.record("member_id", tracing::field::display(member_id));
    }
    match event {
        Event::ReviewSubmitted { decision, .. } => {
            span.record("verdict", decision.as_str());
        }
        Event::ResultDeliveryFailed {
            surface, status, ..
        } => {
            span.record("surface", closed(SURFACES, surface));
            span.record("outcome", closed(OUTCOMES, status));
        }
        _ => {}
    }
}

/// Record a stage that has no event of its own: an acknowledged claim, or a
/// result delivery that landed.
pub fn observe_stage(
    stage: ThreadStage,
    thread_id: ThreadId,
    workspace_id: Option<WorkspaceId>,
    surface: Option<&str>,
) {
    let span = stage_span(stage.as_str(), thread_id);
    if let Some(workspace_id) = workspace_id {
        span.record("workspace_id", tracing::field::display(workspace_id));
    }
    if let Some(surface) = surface {
        span.record("surface", closed(SURFACES, surface));
    }
    if let ThreadStage::Delivered = stage {
        span.record("outcome", "delivered");
    }
}

/// A stage recorded directly rather than from an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStage {
    /// The holder acknowledged its claim and started work.
    Acknowledge,
    /// A result delivery landed on its surface.
    Delivered,
}

impl ThreadStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Acknowledge => "acknowledge",
            Self::Delivered => "delivery",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use maidan_types::{ChannelId, MemberId, ReviewDecision};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::Subscriber;
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

    use super::*;

    #[derive(Clone, Default)]
    struct Seen {
        keys: Arc<Mutex<Vec<String>>>,
        values: Arc<Mutex<Vec<String>>>,
    }

    struct Fields<'a>(&'a Seen);

    impl Visit for Fields<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0
                .values
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("{value:?}"));
            self.0
                .keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(field.name().to_string());
        }
    }

    impl<S: Subscriber> Layer<S> for Seen {
        fn on_new_span(&self, attrs: &Attributes<'_>, _: &Id, _: Context<'_, S>) {
            self.keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(attrs.fields().iter().map(|f| f.name().to_string()));
            attrs.record(&mut Fields(self));
        }

        fn on_record(&self, _: &Id, values: &Record<'_>, _: Context<'_, S>) {
            values.record(&mut Fields(self));
        }
    }

    fn capture(run: impl FnOnce()) -> Seen {
        let seen = Seen::default();
        let subscriber = tracing_subscriber::registry().with(seen.clone());
        tracing::subscriber::with_default(subscriber, run);
        seen
    }

    fn events(thread_id: ThreadId) -> Vec<Event> {
        let workspace_id = WorkspaceId::new();
        let channel_id = ChannelId::new();
        vec![
            Event::ThreadResultSet {
                occurred_at: Default::default(),
                workspace_id,
                channel_id,
                thread_id,
                produced_by: MemberId::new(),
            },
            Event::ReviewSubmitted {
                occurred_at: Default::default(),
                workspace_id,
                channel_id,
                thread_id,
                reviewer_id: MemberId::new(),
                actor_id: None,
                decision: ReviewDecision::RequestChanges,
                sent_back: true,
                worker_id: None,
            },
            Event::ResultDeliveryFailed {
                occurred_at: Default::default(),
                workspace_id,
                channel_id,
                thread_id,
                surface: "secret-surface".into(),
                selector: "private-org/private-repo#7".into(),
                status: "failed".into(),
                last_error: Some("token sk-live-hunter2 rejected".into()),
            },
        ]
    }

    #[test]
    fn every_stage_span_attribute_key_is_on_the_allowlist() {
        let thread_id = ThreadId::new();
        let seen = capture(|| {
            for event in events(thread_id) {
                observe_event(&event);
            }
            observe_stage(ThreadStage::Acknowledge, thread_id, None, None);
            observe_stage(
                ThreadStage::Delivered,
                thread_id,
                Some(WorkspaceId::new()),
                Some("github"),
            );
        });
        let keys = seen.keys.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(!keys.is_empty(), "no stage span was recorded");
        for key in keys.iter() {
            assert!(
                SPAN_ATTRIBUTE_KEYS.contains(&key.as_str()),
                "span attribute `{key}` is not on the allowlist"
            );
        }
    }

    #[test]
    fn free_text_an_event_carries_never_reaches_a_span() {
        let seen = capture(|| {
            for event in events(ThreadId::new()) {
                observe_event(&event);
            }
        });
        let values = seen.values.lock().unwrap_or_else(PoisonError::into_inner);
        for value in values.iter() {
            for secret in ["secret-surface", "private-repo", "hunter2"] {
                assert!(!value.contains(secret), "span value `{value}` leaks text");
            }
        }
        assert!(values.iter().any(|v| v.contains("other")));
        assert!(values.iter().any(|v| v.contains("request_changes")));
    }

    #[test]
    fn a_threads_trace_id_is_its_uuid() {
        let thread_id = ThreadId::new();
        let trace = thread_trace(thread_id);
        assert_eq!(trace.trace_id(), thread_id.0.as_bytes());
        assert_eq!(trace, thread_trace(thread_id));
    }

    #[test]
    fn events_that_mark_no_stage_record_no_span() {
        let seen = capture(|| {
            observe_event(&Event::ThreadAttributesSet {
                occurred_at: Default::default(),
                workspace_id: WorkspaceId::new(),
                channel_id: ChannelId::new(),
                thread_id: ThreadId::new(),
                attributes: Default::default(),
                author_family: None,
                actor_id: MemberId::new(),
            });
        });
        assert!(seen
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty());
    }
}
