//! Member inbox + mention tool handlers (Cluster 149). Lets an MCP-only agent
//! discover it was @mentioned — the read side previously reachable over HTTP
//! only (`/members/:id/mentions`, `/members/:id/inbox`). Cluster 196 adds
//! `wait_for_mention`, a blocking long-poll over the event bus.

use std::{collections::HashSet, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use futures::StreamExt;
use maidan_auth::AuthContext;
use maidan_bus::BusItem;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

/// Default long-poll window, and the ceiling a caller may request.
const DEFAULT_WAIT_MS: i64 = 30_000;
const MAX_WAIT_MS: i64 = 300_000;

/// Default + max list size, mirroring the context tools' clamp.
fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 500)
}

#[derive(Deserialize)]
struct MemberLimitArgs {
    member_id: uuid::Uuid,
    #[serde(default)]
    limit: Option<i64>,
}

pub(super) async fn list_mentions(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: MemberLimitArgs = serde_json::from_value(args.clone())?;
    let mentions = store
        .list_mentions_for_member(MemberId(a.member_id), clamp_limit(a.limit))
        .await?;
    Ok(content_json(&mentions))
}

pub(super) async fn get_inbox(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: MemberLimitArgs = serde_json::from_value(args.clone())?;
    let inbox = store
        .list_member_inbox(MemberId(a.member_id), clamp_limit(a.limit))
        .await?;
    Ok(content_json(&inbox))
}

#[derive(Deserialize)]
struct MarkInboxReadArgs {
    member_id: uuid::Uuid,
    /// Advance the member's read cursor through this instant (RFC 3339).
    read_through: DateTime<Utc>,
}

pub(super) async fn mark_inbox_read(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MarkInboxReadArgs = serde_json::from_value(args.clone())?;
    store
        .advance_inbox_last_read_at(MemberId(a.member_id), a.read_through)
        .await?;
    let inbox = store.list_member_inbox(MemberId(a.member_id), 50).await?;
    Ok(content_json(&inbox))
}

#[derive(Deserialize)]
struct WaitForMentionArgs {
    member_id: uuid::Uuid,
    /// Long-poll window in milliseconds (default 30 000, clamped to 1 000–300 000).
    #[serde(default)]
    timeout_ms: Option<i64>,
}

/// Block until `member_id` is next @mentioned, or the timeout lapses (Cluster
/// 196). Subscribes to the event bus filtered to this member's
/// `MentionRecorded` events and returns the first one whose thread the caller
/// can access; returns `null` on timeout. This is a **live** primitive — it only
/// sees mentions recorded *after* the call subscribes, so drain existing ones
/// with `get_inbox`/`list_mentions` first (the `GET /mcp/stream` SSE transport
/// is the at-least-once, resumable alternative when a missed mention is
/// unacceptable).
pub(super) async fn wait_for_mention(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForMentionArgs = serde_json::from_value(args.clone())?;
    wait_for_member_event(
        server,
        auth,
        MemberId(a.member_id),
        HashSet::from([EventKind::MentionRecorded]),
        a.timeout_ms,
    )
    .await
}

/// Shared member-addressed long-poll: block until an event of one of `kinds`
/// addressed to `member_id` arrives, or the timeout lapses. Returns the triggering
/// event (unless it lives in a thread the caller can't access — then keep waiting),
/// or `null` on timeout. Backs `wait_for_mention` (Cluster 196) and
/// `wait_for_notification` (Cluster 240). **Live** — only sees events after
/// subscribe.
async fn wait_for_member_event(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    member_id: MemberId,
    kinds: HashSet<EventKind>,
    timeout_ms: Option<i64>,
) -> Result<Value, McpError> {
    let Some(bus) = server.event_bus.as_ref() else {
        return Err(McpError::InvalidParams(
            "waiting requires an event bus".into(),
        ));
    };
    let wait = timeout_ms.unwrap_or(DEFAULT_WAIT_MS).clamp(1, MAX_WAIT_MS);

    let filter = EventFilter {
        member_id: Some(member_id),
        kinds: Some(kinds),
        ..EventFilter::default()
    };
    let mut stream = bus
        .subscribe(filter)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;

    let store = server.store.as_ref();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait as u64);
    loop {
        let item = match tokio::time::timeout_at(deadline, stream.next()).await {
            // Timed out or the bus closed → nothing arrived in the window.
            Err(_) | Ok(None) => return Ok(content_json(&Value::Null)),
            Ok(Some(item)) => item,
        };
        // A lag marker means the buffer overflowed; keep waiting rather than
        // reporting a false timeout (bounded by the same deadline).
        let BusItem::Event(envelope) = item else {
            continue;
        };
        // Addressed to this member, but if it lives in a thread the caller can't
        // access, don't reveal it — keep waiting.
        if !auth.bypass {
            if let Some(tid) = envelope.event.thread_id() {
                if !maidan_auth::can_access_thread(store, auth, tid).await? {
                    continue;
                }
            }
        }
        return Ok(content_json(&envelope.event));
    }
}

/// The event kinds the notification router (Cluster 238) turns into per-recipient
/// notifications — the set `wait_for_notification` waits on. Grows as Arc H routes
/// more kinds; keep in step with `notification_router::route_event`.
fn notifiable_kinds() -> HashSet<EventKind> {
    HashSet::from([EventKind::MentionRecorded])
}

#[derive(Deserialize)]
struct ListNotificationsArgs {
    member_id: uuid::Uuid,
    #[serde(default)]
    unread_only: bool,
    #[serde(default)]
    limit: Option<i64>,
}

/// A member's per-recipient notifications (Cluster 240), newest first, optionally
/// unread-only — the MCP twin of `GET /members/:id/notifications`. Mirrors the
/// sibling inbox tools (`get_inbox`/`list_mentions`): a member-scoped read, no
/// per-channel filter.
pub(super) async fn list_notifications(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListNotificationsArgs = serde_json::from_value(args.clone())?;
    let notes = store
        .list_notifications(MemberId(a.member_id), a.unread_only, clamp_limit(a.limit))
        .await?;
    Ok(content_json(&notes))
}

#[derive(Deserialize)]
struct MemberIdArg {
    member_id: uuid::Uuid,
}

/// A member's unread-notification badge count (Cluster 240).
pub(super) async fn get_unread_count(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArg = serde_json::from_value(args.clone())?;
    let count = store
        .unread_notification_count(MemberId(a.member_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "count": count })))
}

#[derive(Deserialize)]
struct MarkNotificationReadArgs {
    member_id: uuid::Uuid,
    notification_id: uuid::Uuid,
}

/// Mark one of a member's notifications read (Cluster 240). Recipient-scoped in the
/// store, so `{marked: false}` when the id isn't this member's.
pub(super) async fn mark_notification_read(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MarkNotificationReadArgs = serde_json::from_value(args.clone())?;
    let marked = store
        .mark_notification_read(MemberId(a.member_id), NotificationId(a.notification_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "marked": marked })))
}

/// Block until `member_id` gets a new notification-worthy event, or the timeout
/// lapses (Cluster 240). The general form of `wait_for_mention`: it waits on the
/// event kinds the notification router acts on (today: mentions), filtered to this
/// member, and returns the triggering event (RBAC-checked) or `null`. **Live** —
/// drain existing notifications with `list_notifications`/`get_unread_count` first
/// (the durable ledger + `GET /mcp/stream` are the at-least-once alternatives).
pub(super) async fn wait_for_notification(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForMentionArgs = serde_json::from_value(args.clone())?;
    wait_for_member_event(
        server,
        auth,
        MemberId(a.member_id),
        notifiable_kinds(),
        a.timeout_ms,
    )
    .await
}

#[derive(Deserialize)]
struct SetNotificationPrefArgs {
    member_id: uuid::Uuid,
    /// The event kind to (un)mute, snake_case (e.g. `mention_recorded`).
    kind: String,
    muted: bool,
}

/// Set a member's mute preference for an event kind (Cluster 243) — the MCP twin of
/// `PUT /members/:id/notification-prefs`. The router skips writing notifications of a
/// muted kind for this member.
pub(super) async fn set_notification_pref(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetNotificationPrefArgs = serde_json::from_value(args.clone())?;
    let kind = EventKind::parse(&a.kind)
        .ok_or_else(|| McpError::InvalidParams(format!("unknown event kind: {}", a.kind)))?;
    let pref = store
        .set_notification_pref(MemberId(a.member_id), kind, a.muted)
        .await?;
    Ok(content_json(&pref))
}

#[derive(Deserialize)]
struct ListNotificationPrefsArgs {
    member_id: uuid::Uuid,
}

/// A member's notification preferences (Cluster 243).
pub(super) async fn list_notification_prefs(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListNotificationPrefsArgs = serde_json::from_value(args.clone())?;
    let prefs = store.list_notification_prefs(MemberId(a.member_id)).await?;
    Ok(content_json(&prefs))
}
