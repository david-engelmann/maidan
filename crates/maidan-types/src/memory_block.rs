//! Attachable labeled memory as room objects (Cluster 373, Wave 2 #21, H11).
//!
//! A [`MemoryBlock`] is a Letta-shaped unit of shared, mutable memory —
//! `{label, description, limit, read_only, value}` — that lives in a workspace
//! and can be **attached** to a thread (a "room"). Blocks are the mechanism by
//! which a parent thread watches a child's result block **without a nested
//! runtime**: they share the block, and the child's write is the parent's read.
//!
//! It is deliberately **not a transcript** (no append log — `value` is replaced
//! whole, last-writer-wins) and **not RAG** (no embedding or similarity search —
//! a block is addressed by its `label`). `char_limit` mirrors Letta's block
//! limit; `read_only` freezes a block against further writes; `owner_id` records
//! who owns it.
//!
//! The validation helpers here are pure so the store and route layers share one
//! definition of "does this value fit" / "is this a valid label".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{MemberId, MemoryBlockId, WorkspaceId};

/// A labeled memory block. `value` is the whole content (a full-rewrite,
/// last-writer-wins field); `char_limit` (Letta's `limit`) bounds its length in
/// Unicode scalar values when set; `read_only` refuses writes; `owner_id` is the
/// member who owns the block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemoryBlock {
    pub id: MemoryBlockId,
    pub workspace_id: WorkspaceId,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_limit: Option<i64>,
    pub read_only: bool,
    pub value: String,
    pub owner_id: MemberId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A new memory block to persist. Creation is concurrent-safe on
/// `(workspace_id, label)` — re-creating an existing label is a no-op (the
/// existing block is returned), so two racers converge on one block.
#[derive(Debug, Clone)]
pub struct NewMemoryBlock {
    pub workspace_id: WorkspaceId,
    pub label: String,
    pub description: Option<String>,
    pub char_limit: Option<i64>,
    pub read_only: bool,
    pub value: String,
    pub owner_id: MemberId,
}

/// Whether `value` fits within an optional character limit, counted in Unicode
/// scalar values. `None` (or a negative limit) is unbounded. Pure so the store
/// and the route edge enforce the same bound.
pub fn fits_char_limit(value: &str, char_limit: Option<i64>) -> bool {
    match char_limit {
        Some(limit) if limit >= 0 => value.chars().count() as i64 <= limit,
        _ => true,
    }
}

/// Whether `label` is a valid block label: non-empty, no surrounding whitespace,
/// and within a sane length. A label is the block's within-workspace key, so it
/// must be a stable, trimmed identifier rather than free prose.
pub fn is_valid_block_label(label: &str) -> bool {
    !label.is_empty() && label.len() <= 128 && label.trim() == label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_limit_counts_unicode_scalars_not_bytes() {
        // "é" is 2 bytes but 1 char; a 3-char value fits a limit of 3.
        assert!(fits_char_limit("éé…", Some(3)));
        assert!(!fits_char_limit("éé…!", Some(3)));
    }

    #[test]
    fn char_limit_none_or_negative_is_unbounded() {
        assert!(fits_char_limit("anything at all", None));
        assert!(fits_char_limit("anything at all", Some(-1)));
    }

    #[test]
    fn char_limit_zero_admits_only_empty() {
        assert!(fits_char_limit("", Some(0)));
        assert!(!fits_char_limit("x", Some(0)));
    }

    #[test]
    fn valid_labels_are_trimmed_nonempty_and_bounded() {
        assert!(is_valid_block_label("persona"));
        assert!(is_valid_block_label("child.result"));
        assert!(!is_valid_block_label(""));
        assert!(!is_valid_block_label(" leading"));
        assert!(!is_valid_block_label("trailing "));
        assert!(!is_valid_block_label(&"x".repeat(129)));
    }
}
