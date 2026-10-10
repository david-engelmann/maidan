//! Subscribe / projector cursor freshness.
//!
//! A client that presents an `after_id` pointing into a gap the retention
//! sweeper already deleted must **fail loud** (`CursorTooOld`, HTTP 409,
//! `must_refetch: true`). Silently clamping to the oldest remaining row is the
//! Postel anti-pattern: the client thinks it caught up and never learns it
//! missed events.
//!
//! `after_id == 0` means "start from whatever remains" — a fresh subscriber —
//! and is never too old.

use serde::{Deserialize, Serialize};

use crate::events::{Event, EventFilter, EventKind, StoredEvent};
use crate::ids::{ChannelId, ThreadId, WorkspaceId};

/// Wire body / problem extension when a subscribe or backfill cursor is
/// behind the retained log. `must_refetch` is always `true` — the client
/// must take a snapshot (or HTTP backfill from a known-good point) rather
/// than continue from the stale cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CursorTooOld {
    pub after_id: i64,
    pub oldest_id: i64,
    pub must_refetch: bool,
}

impl CursorTooOld {
    pub fn new(after_id: i64, oldest_id: i64) -> Self {
        Self {
            after_id,
            oldest_id,
            must_refetch: true,
        }
    }
}

/// A subscribe cursor is too old when it points *into* a pruned gap:
/// the next id the client wants (`after_id + 1`) is strictly less than
/// the oldest retained row. Adjacent (`after_id + 1 == oldest`) is fine
/// — that is a normal resume just behind the remaining log.
///
/// `after_id <= 0` (fresh) and an empty log (`oldest_id == None`) are
/// never too old. `after_id == i64::MAX` has no next id, so it is not a
/// gap either: the addition is checked, and a wrapping `+ 1` would call
/// a cursor at the end of the id space too old.
pub fn cursor_is_too_old(after_id: i64, oldest_retained_id: Option<i64>) -> bool {
    match oldest_retained_id {
        Some(oldest) if after_id > 0 => after_id.checked_add(1).is_some_and(|next| next < oldest),
        _ => false,
    }
}

/// Projector subscription shape: the filter a tap / Slack / GitHub projector
/// (or any thick client) uses to backfill then cut over to live. `{workspace,
/// channel?, thread?, types[]}`.
///
/// Empty `types` means every kind. Converted to [`EventFilter`] for the
/// existing matchers; a 409 [`CursorTooOld`] on this shape is a must-refetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectorShape {
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<ChannelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    /// Event kinds this projector cares about. Empty = all kinds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<EventKind>,
}

impl ProjectorShape {
    pub fn workspace(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            channel_id: None,
            thread_id: None,
            types: Vec::new(),
        }
    }

    pub fn to_filter(&self) -> EventFilter {
        let mut filter = EventFilter::workspace(self.workspace_id);
        filter.channel_id = self.channel_id;
        filter.thread_id = self.thread_id;
        if !self.types.is_empty() {
            filter.kinds = Some(self.types.iter().copied().collect());
        }
        filter
    }

    pub fn matches(&self, event: &Event) -> bool {
        self.to_filter().matches(event)
    }

    /// Same shape check against a log row's denormalized columns (HTTP backfill).
    pub fn matches_stored(&self, event: &StoredEvent) -> bool {
        if event.workspace_id != Some(self.workspace_id) {
            return false;
        }
        if let Some(ch) = self.channel_id {
            if event.channel_id != Some(ch) {
                return false;
            }
        }
        if let Some(th) = self.thread_id {
            if event.thread_id != Some(th) {
                return false;
            }
        }
        if !self.types.is_empty() && !self.types.contains(&event.kind) {
            return false;
        }
        true
    }
}

/// Parse a comma-separated `types` query (`message_posted,thread_ready`).
/// Unknown tokens fail — fail loud, no silent drop of a mistyped kind.
pub fn parse_projector_types(s: &str) -> Result<Vec<EventKind>, String> {
    let mut out = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let kind = EventKind::parse(part).ok_or_else(|| format!("unknown event type '{part}'"))?;
        out.push(kind);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ChannelId, ThreadId, WorkspaceId};

    fn ws() -> WorkspaceId {
        WorkspaceId(uuid::Uuid::from_u128(1))
    }

    #[test]
    fn fresh_cursor_is_never_too_old() {
        assert!(!cursor_is_too_old(0, Some(100)));
        assert!(!cursor_is_too_old(0, None));
        assert!(!cursor_is_too_old(-1, Some(100)));
    }

    #[test]
    fn empty_log_is_never_too_old() {
        assert!(!cursor_is_too_old(50, None));
    }

    #[test]
    fn adjacent_resume_is_not_too_old() {
        // Client last saw 99; oldest retained is 100 → next wanted is 100.
        assert!(!cursor_is_too_old(99, Some(100)));
        assert!(!cursor_is_too_old(100, Some(100)));
        assert!(!cursor_is_too_old(150, Some(100)));
    }

    #[test]
    fn pruned_gap_is_too_old() {
        // Events 1..=99 gone; oldest retained is 100; client asks after 50.
        assert!(cursor_is_too_old(50, Some(100)));
        assert!(cursor_is_too_old(1, Some(100)));
        assert!(cursor_is_too_old(98, Some(100)));
    }

    #[test]
    fn cursor_too_old_always_must_refetch() {
        let body = CursorTooOld::new(50, 100);
        assert!(body.must_refetch);
        assert_eq!(body.after_id, 50);
        assert_eq!(body.oldest_id, 100);
    }

    #[test]
    fn a_cursor_at_the_end_of_the_id_space_is_not_a_gap() {
        assert!(!cursor_is_too_old(i64::MAX, Some(1)));
        assert!(!cursor_is_too_old(i64::MAX, Some(i64::MAX)));
        assert!(!cursor_is_too_old(i64::MAX, None));
    }

    #[test]
    fn projector_shape_round_trips_and_filters() {
        let shape = ProjectorShape {
            workspace_id: ws(),
            channel_id: Some(ChannelId(uuid::Uuid::from_u128(2))),
            thread_id: Some(ThreadId(uuid::Uuid::from_u128(3))),
            types: vec![EventKind::MessagePosted, EventKind::ThreadReady],
        };
        let json = serde_json::to_value(&shape).unwrap();
        assert_eq!(json["workspace_id"], uuid::Uuid::from_u128(1).to_string());
        assert_eq!(
            json["types"],
            serde_json::json!(["message_posted", "thread_ready"])
        );
        let back: ProjectorShape = serde_json::from_value(json).unwrap();
        assert_eq!(back, shape);
        let filter = shape.to_filter();
        assert_eq!(filter.workspace_id, Some(ws()));
        assert_eq!(filter.channel_id, shape.channel_id);
        assert_eq!(filter.thread_id, shape.thread_id);
        assert_eq!(filter.kinds.as_ref().map(|s| s.len()), Some(2));

        let stored = StoredEvent {
            id: 1,
            lsn: 1,
            kind: EventKind::MessagePosted,
            workspace_id: Some(ws()),
            channel_id: shape.channel_id,
            thread_id: shape.thread_id,
            payload: serde_json::json!({}),
            occurred_at: chrono::Utc::now(),
            prev_hash: crate::genesis_hash(),
            content_hash: crate::content_hash(&serde_json::json!({})).unwrap(),
            content_key: None,
            trace: None,
        };
        assert!(shape.matches_stored(&stored));
        let other = StoredEvent {
            kind: EventKind::MemberJoined,
            ..stored.clone()
        };
        assert!(!shape.matches_stored(&other));
    }

    #[test]
    fn empty_types_means_all_kinds() {
        let shape = ProjectorShape::workspace(ws());
        assert!(shape.to_filter().kinds.is_none());
        let json = serde_json::to_value(&shape).unwrap();
        assert!(json.get("types").is_none());
        assert!(json.get("channel_id").is_none());
    }

    #[test]
    fn parse_projector_types_fails_loud_on_unknown() {
        assert_eq!(
            parse_projector_types("message_posted,thread_ready").unwrap(),
            vec![EventKind::MessagePosted, EventKind::ThreadReady]
        );
        assert!(parse_projector_types("message_posted,not_a_kind").is_err());
        assert!(parse_projector_types("").unwrap().is_empty());
        assert!(parse_projector_types("  ,  ").unwrap().is_empty());
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// The gap rule over every cursor and every retained floor.
    /// A fresh cursor, an empty log, an adjacent resume, and a cursor
    /// already at `i64::MAX` are not too old. A next id strictly behind
    /// the floor is.
    #[kani::proof]
    fn a_pruned_gap_is_the_only_cursor_that_is_too_old() {
        let after: i64 = kani::any();
        let oldest: i64 = kani::any();
        let retained: bool = kani::any();
        let floor = retained.then_some(oldest);
        let too_old = cursor_is_too_old(after, floor);

        if after <= 0 || floor.is_none() {
            assert!(!too_old);
        }
        match after.checked_add(1) {
            Some(next) if after > 0 => {
                if let Some(oldest) = floor {
                    assert_eq!(too_old, next < oldest);
                    if next == oldest || after >= oldest {
                        assert!(!too_old);
                    }
                }
            }
            Some(_) => assert!(!too_old),
            None => assert!(!too_old),
        }
    }

    /// Catch-up uses the same rule, negated: allowed exactly when the
    /// cursor is not sitting in a pruned gap.
    #[kani::proof]
    fn catch_up_is_allowed_exactly_when_the_cursor_is_not_too_old() {
        let after: i64 = kani::any();
        let oldest: i64 = kani::any();
        let retained: bool = kani::any();
        let floor = retained.then_some(oldest);
        assert_eq!(
            crate::log_snapshot::catch_up_allowed(after, floor),
            !cursor_is_too_old(after, floor)
        );
    }

    #[kani::proof]
    fn a_too_old_body_always_says_to_refetch() {
        let after: i64 = kani::any();
        let oldest: i64 = kani::any();
        let body = CursorTooOld::new(after, oldest);
        assert!(body.must_refetch);
        assert_eq!(body.after_id, after);
        assert_eq!(body.oldest_id, oldest);
    }
}
