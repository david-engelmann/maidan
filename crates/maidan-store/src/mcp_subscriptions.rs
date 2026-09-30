//! Stateless MCP resource subscriptions: the types the store trades in.
//!
//! A stateless MCP caller (`POST /mcp`, or the streamable POST from
//! `2025-03-26` on) has no protocol session, so its credential is its
//! identity, and behind a load balancer its `resources/subscribe` and its
//! notification listener may land on different replicas. The subscription is
//! therefore kept in the database, where the replica holding the listener
//! finds it when an update fans out over NOTIFY. Session-bound subscriptions
//! (a `2024-11-05` session, stdio) live and die with their process and stay
//! in memory.

use chrono::{DateTime, Utc};
use maidan_types::{MemberId, WorkspaceId};

/// A stateless caller's subscription to one resource.
#[derive(Debug, Clone)]
pub struct NewMcpSubscription {
    /// The caller's principal, spelled as a key; a listener looks its
    /// subscriptions up by it.
    pub subscriber: String,
    /// `None` only for an auth-disabled caller.
    pub workspace_id: Option<WorkspaceId>,
    pub member_id: Option<MemberId>,
    pub uri: String,
    /// When the subscriber's subscriptions lapse unless a listener keeps
    /// extending them.
    pub expires_at: DateTime<Utc>,
}

/// One live subscription an update may be delivered to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, sqlx::FromRow)]
pub struct McpSubscriptionWatch {
    pub subscriber: String,
    pub uri: String,
}
