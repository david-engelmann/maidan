//! Integrity explorer types (Cluster 394, Wave 3 #34).
//!
//! Three read surfaces over existing rows — no new table:
//!
//! - [`TombstoneRecord`]: deleted/tombstoned messages, including hard-purged
//!   reconstructions from `MessageTombstoned` events.
//! - [`MessageBacklinks`]: incoming pointers at a message (`RelationKind`
//!   reverse edges plus pins / reactions / votes).
//! - [`KindCensus`]: `EventKind` distribution for a workspace/scope.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::events::EventKind;
use crate::ids::{ChannelId, MemberId, MessageId, ThreadId, WorkspaceId};
use crate::models::{Pin, Reaction, Reference, Vote};

/// Default page size for the tombstone explorer.
pub const TOMBSTONE_LIST_DEFAULT: i64 = 100;
/// Hard clamp for the tombstone explorer (same band as other list reads).
pub const TOMBSTONE_LIST_MAX: i64 = 500;

/// What kind of entity a [`TombstoneRecord`] describes.
///
/// Only messages are produced today: threads, channels, and members carry a
/// `tombstoned_at` column but have no tombstone API. The enum is closed so a
/// later entity kind is an additive variant, not a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TombstoneEntityKind {
    Message,
}

/// One deleted or tombstoned message in the explorer.
///
/// Soft-delete (`retained = true`) keeps the row and clears `body`/`content`.
/// Hard purge (`retained = false`) reconstructs the record from the
/// `MessageTombstoned` event — the body is gone either way. This surface is
/// honest about deletion; it does not restore content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TombstoneRecord {
    pub entity_kind: TombstoneEntityKind,
    pub id: uuid::Uuid,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    /// Author of the original message. `None` after a hard purge (the row is gone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<MemberId>,
    pub tombstoned_at: DateTime<Utc>,
    /// Soft-delete row still present. `false` after `purge_message`.
    pub retained: bool,
    /// Event-log id of the `MessageTombstoned` row when the explorer joined it
    /// (hard-purged reconstructions always have one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_log_id: Option<i64>,
}

/// Incoming pointers at one message — the backlink index.
///
/// `references` is Cluster 320 `list_references_to(Message, id)` (`RelationKind`
/// reverse edges, including `seeded_from`). Pins, reactions, and votes are the
/// other existing tables that point *at* the message. Mentions are outgoing
/// (the message points at members) and are not included.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MessageBacklinks {
    pub message_id: MessageId,
    pub references: Vec<Reference>,
    pub pins: Vec<Pin>,
    pub reactions: Vec<Reaction>,
    pub votes: Vec<Vote>,
}

/// One `EventKind` bucket in a [`KindCensus`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KindCount {
    pub kind: EventKind,
    pub count: i64,
}

/// EventKind distribution for a workspace (optionally narrowed to a channel
/// or thread). Zero-count kinds are omitted; [`KindCensus::total`] is the
/// denominator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KindCensus {
    pub workspace_id: WorkspaceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<ChannelId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    pub total: i64,
    pub counts: Vec<KindCount>,
}

/// Clamp a tombstone-list `limit` into `1..=TOMBSTONE_LIST_MAX`.
pub fn clamp_tombstone_limit(limit: Option<i64>) -> i64 {
    limit
        .unwrap_or(TOMBSTONE_LIST_DEFAULT)
        .clamp(1, TOMBSTONE_LIST_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::WorkspaceId;
    use uuid::Uuid;

    #[test]
    fn tombstone_entity_kind_wire_is_message() {
        let json = serde_json::to_string(&TombstoneEntityKind::Message).unwrap();
        assert_eq!(json, "\"message\"");
        let back: TombstoneEntityKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TombstoneEntityKind::Message);
    }

    #[test]
    fn clamp_tombstone_limit_defaults_and_clamps() {
        assert_eq!(clamp_tombstone_limit(None), TOMBSTONE_LIST_DEFAULT);
        assert_eq!(clamp_tombstone_limit(Some(0)), 1);
        assert_eq!(
            clamp_tombstone_limit(Some(TOMBSTONE_LIST_MAX + 10)),
            TOMBSTONE_LIST_MAX
        );
        assert_eq!(clamp_tombstone_limit(Some(12)), 12);
    }

    #[test]
    fn kind_census_omits_empty_scope_ids() {
        let census = KindCensus {
            workspace_id: WorkspaceId(Uuid::nil()),
            channel_id: None,
            thread_id: None,
            total: 2,
            counts: vec![KindCount {
                kind: EventKind::MessagePosted,
                count: 2,
            }],
        };
        let v = serde_json::to_value(&census).unwrap();
        assert!(v.get("channel_id").is_none());
        assert!(v.get("thread_id").is_none());
        assert_eq!(v["counts"][0]["kind"], "message_posted");
        assert_eq!(v["total"], 2);
    }
}
