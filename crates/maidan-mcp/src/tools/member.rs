//! Member inbox + mention tool handlers. Lets an MCP-only agent discover it was
//! @mentioned — the read side previously reachable over HTTP only
//! (`/members/:id/mentions`, `/members/:id/inbox`), plus `wait_for_mention`, a
//! blocking long-poll over the event bus.

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
/// Page size for the lookback replay over the durable event log.
const LOOKBACK_BATCH: i64 = 256;

/// Default + max list size, mirroring the context tools' clamp.
fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 500)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberLimitArgs {
    member_id: uuid::Uuid,
    #[serde(default)]
    limit: Option<i64>,
}

pub(super) async fn list_mentions(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: MemberLimitArgs = crate::tools::parse_args(args)?;
    let mentions = store
        .list_mentions_for_member(MemberId(a.member_id), clamp_limit(a.limit))
        .await?;
    Ok(content_json(&mentions))
}

pub(super) async fn get_inbox(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: MemberLimitArgs = crate::tools::parse_args(args)?;
    let inbox = store
        .list_member_inbox(MemberId(a.member_id), clamp_limit(a.limit))
        .await?;
    Ok(content_json(&inbox))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkInboxReadArgs {
    member_id: uuid::Uuid,
    /// Advance the member's read cursor through this instant (RFC 3339).
    read_through: DateTime<Utc>,
}

pub(super) async fn mark_inbox_read(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MarkInboxReadArgs = crate::tools::parse_args(args)?;
    store
        .advance_inbox_last_read_at(MemberId(a.member_id), a.read_through)
        .await?;
    let inbox = store.list_member_inbox(MemberId(a.member_id), 50).await?;
    Ok(content_json(&inbox))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitForMentionArgs {
    member_id: uuid::Uuid,
    /// Long-poll window in milliseconds (default 30 000, clamped to 1 000–300 000).
    #[serde(default)]
    timeout_ms: Option<i64>,
    /// Lookback anchor: the caller's high-water `log_id` from its last drain.
    /// When set, the wait first replays the durable log for a matching event
    /// with `log_id > since_log_id` before parking on the live stream — so a
    /// signal that fired in the gap between the drain and this subscribe is not
    /// missed. Omit for the pure-live behaviour.
    #[serde(default)]
    since_log_id: Option<i64>,
}

/// Block until `member_id` is next @mentioned, or the timeout lapses.
/// Subscribes to the event bus filtered to this member's `MentionRecorded`
/// events and returns the first one whose thread the caller can access; returns
/// `null` on timeout. This is a **live** primitive — it only sees mentions
/// recorded *after* the call subscribes, so drain existing ones with
/// `get_inbox`/`list_mentions` first (the `GET /mcp/stream` SSE transport is
/// the at-least-once, resumable alternative when a missed mention is
/// unacceptable).
pub(super) async fn wait_for_mention(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForMentionArgs = crate::tools::parse_args(args)?;
    wait_for_member_event(
        server,
        auth,
        MemberId(a.member_id),
        HashSet::from([EventKind::MentionRecorded]),
        a.timeout_ms,
        a.since_log_id,
    )
    .await
}

/// Shared member-addressed long-poll: block until an event of one of `kinds`
/// addressed to `member_id` arrives, or the timeout lapses. Returns the
/// triggering event (unless it lives in a thread the caller can't access — then
/// keep waiting), or `null` on timeout. Backs `wait_for_mention` and
/// `wait_for_notification`. **Live** — only sees events after subscribe.
async fn wait_for_member_event(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    member_id: MemberId,
    kinds: HashSet<EventKind>,
    timeout_ms: Option<i64>,
    since_log_id: Option<i64>,
) -> Result<Value, McpError> {
    let Some(bus) = server.event_bus.as_ref() else {
        return Err(McpError::InvalidParams(
            "waiting requires an event bus".into(),
        ));
    };
    let wait = timeout_ms.unwrap_or(DEFAULT_WAIT_MS).clamp(1, MAX_WAIT_MS);

    let filter = EventFilter {
        member_id: Some(member_id),
        kinds: Some(kinds.clone()),
        ..EventFilter::default()
    };
    // Subscribe BEFORE the lookback so there is no gap: an event that commits
    // between the lookback query and now is caught live; one that committed
    // before the query is caught by the lookback. Any overlap resolves to a
    // single return (the lookback wins and the live stream is dropped).
    let mut stream = bus
        .subscribe(filter)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;

    let store = server.store.as_ref();
    // Where a replay after a lag starts (see `thread::next_visible_event`).
    let mut cursor = match since_log_id {
        Some(since) => since,
        None => store.max_event_id().await?,
    };

    if let Some(since) = since_log_id
        && let Some(event) = lookback_member_event(store, auth, member_id, &kinds, since).await?
    {
        return Ok(content_json(&event));
    }

    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait as u64);
    loop {
        let item = match tokio::time::timeout_at(deadline, stream.next()).await {
            // Timed out or the bus closed → nothing arrived in the window.
            Err(_) | Ok(None) => return Ok(content_json(&Value::Null)),
            Ok(Some(item)) => item,
        };
        let envelope = match item {
            BusItem::Event(envelope) => envelope,
            // The buffer overflowed and dropped events: read what was dropped
            // from the log, then keep waiting on the same deadline.
            BusItem::Lagged { .. } => {
                if let Some(event) =
                    lookback_member_event(store, auth, member_id, &kinds, cursor).await?
                {
                    return Ok(content_json(&event));
                }
                continue;
            }
        };
        cursor = cursor.max(envelope.log_id);
        // Addressed to this member, but if it lives in a thread the caller can't
        // access, don't reveal it — keep waiting.
        if !auth.bypass
            && let Some(tid) = envelope.event.thread_id()
            && !maidan_auth::can_access_thread(store, auth, tid).await?
        {
            continue;
        }
        return Ok(content_json(&envelope.event));
    }
}

/// Replay the durable event log for the earliest event of one of `kinds`
/// addressed to `member_id` with `log_id > since`. Returns the deserialized
/// `Event`, RBAC-filtered like the live path (a match in a thread the caller
/// can't access is skipped, not revealed), or `None` if the log has no such
/// event. Scoped to `auth.workspace_id` — member-addressed events live there.
async fn lookback_member_event(
    store: &dyn Store,
    auth: &AuthContext,
    member_id: MemberId,
    kinds: &HashSet<EventKind>,
    since: i64,
) -> Result<Option<Event>, McpError> {
    let mut after = since;
    loop {
        let batch = store
            .list_events_after(auth.workspace_id, after, LOOKBACK_BATCH)
            .await?;
        let drained = (batch.len() as i64) < LOOKBACK_BATCH;
        for stored in &batch {
            after = stored.id;
            if !kinds.contains(&stored.kind) {
                continue;
            }
            let event: Event = stored
                .opened_event()
                .map_err(|e| McpError::Internal(e.to_string()))?;
            if event.member_id() != Some(member_id) {
                continue;
            }
            if !auth.bypass
                && let Some(tid) = event.thread_id()
                && !maidan_auth::can_access_thread(store, auth, tid).await?
            {
                continue;
            }
            return Ok(Some(event));
        }
        if drained {
            return Ok(None);
        }
    }
}

/// How often a notification wait reads the ledger when no event wakes it.
/// The bus says something happened; only the ledger says it reached this
/// member, and a replica that is not this one may have written the row.
const NOTIFICATION_POLL: Duration = Duration::from_secs(2);
/// How long a notification wait gives the router to write the row after an
/// event wakes it, which also folds a burst of events into one read.
const NOTIFICATION_SETTLE: Duration = Duration::from_millis(150);
/// Unread rows a notification wait reads at once.
const NOTIFICATION_SCAN: i64 = 500;
/// The longest `reason` a digest line carries.
const REASON_MAX_CHARS: usize = 200;

/// A notification with what one digest line needs: the thread's title (the
/// item's own, for an intake proposal) and a short reason read from the
/// event that caused it. Both are absent when the caller can no longer read
/// the thread or the event's words were erased.
#[derive(serde::Serialize)]
struct NotificationLine {
    #[serde(flatten)]
    notification: Notification,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Thread titles and access, read once per thread for a whole listing.
#[derive(Default)]
struct ThreadCache(std::collections::HashMap<ThreadId, Option<Option<String>>>);

impl ThreadCache {
    /// `None` when the caller cannot read the thread; otherwise its title.
    async fn title(
        &mut self,
        store: &dyn Store,
        auth: &AuthContext,
        thread_id: ThreadId,
    ) -> Result<Option<Option<String>>, McpError> {
        if let Some(cached) = self.0.get(&thread_id) {
            return Ok(cached.clone());
        }
        let readable =
            auth.bypass || maidan_auth::can_access_thread(store, auth, thread_id).await?;
        let title = if readable {
            Some(store.get_thread(thread_id).await.ok().and_then(|t| t.title))
        } else {
            None
        };
        self.0.insert(thread_id, title.clone());
        Ok(title)
    }
}

async fn notification_line(
    store: &dyn Store,
    auth: &AuthContext,
    threads: &mut ThreadCache,
    notification: Notification,
) -> Result<NotificationLine, McpError> {
    let mut title = None;
    if let Some(thread_id) = notification.thread_id {
        match threads.title(store, auth, thread_id).await? {
            Some(thread_title) => title = thread_title,
            // Routed when the member could read it; not any more, so the row
            // says only what it already said.
            None => {
                return Ok(NotificationLine {
                    notification,
                    title: None,
                    reason: None,
                });
            }
        }
    }
    let mut reason = None;
    if let Ok(stored) = store.get_stored_event(notification.source_log_id).await
        && stored.workspace_id == Some(notification.workspace_id)
        && let Ok(event) = stored.opened_event()
    {
        if let Event::IntakeProposed {
            title: item_title, ..
        } = &event
            && title.is_none()
        {
            title = Some(item_title.clone());
        }
        reason = attention_reason(&event).map(|r| clip(&r));
    }
    Ok(NotificationLine {
        notification,
        title,
        reason,
    })
}

fn clip(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match one_line.char_indices().nth(REASON_MAX_CHARS) {
        Some((cut, _)) => format!("{}…", &one_line[..cut]),
        None => one_line,
    }
}

fn joined(items: &[String]) -> String {
    items.join(", ")
}

/// Why a notification's event wants someone's attention, in a few words.
fn attention_reason(event: &Event) -> Option<String> {
    Some(match event {
        Event::MentionRecorded { .. } => "mentioned you".into(),
        Event::StatusDeclared { status, note, .. } => format!("{}: {note}", status.as_str()),
        Event::ClaimStalled { reason, .. } => format!("claim stalled ({})", reason.as_str()),
        Event::ClaimUnacknowledged { .. } => "claim not acknowledged".into(),
        Event::ClaimExpired { .. } => "claim expired".into(),
        Event::ClaimFailed { reason, .. } => format!("run stopped over its {reason} budget"),
        Event::ThreadHandoff {
            rounds,
            repo,
            pr_number,
            to_engine,
            ..
        } => format!(
            "not converging after {rounds} rounds on {repo}#{pr_number}; handed to {}",
            to_engine.as_str()
        ),
        Event::ThreadNotice {
            notice,
            repo,
            pr_number,
            failing_jobs,
            reasons,
            ..
        } => match notice {
            ThreadNoticeKind::CiFailed => {
                format!("CI failed on {repo}#{pr_number}: {}", joined(failing_jobs))
            }
            ThreadNoticeKind::GateBlocked => {
                format!("{repo}#{pr_number} cannot merge: {}", joined(reasons))
            }
            other => format!("{} on {repo}#{pr_number}", other.as_str()),
        },
        Event::ThreadBlocked { reason, note, .. } => match note {
            Some(note) => format!("blocked ({}): {note}", reason.as_str()),
            None => format!("blocked ({})", reason.as_str()),
        },
        Event::ResultDeliveryFailed {
            surface,
            selector,
            status,
            last_error,
            ..
        } => match last_error {
            Some(error) => format!("delivery to {surface} {selector} {status}: {error}"),
            None => format!("delivery to {surface} {selector} {status}"),
        },
        Event::IntakeProposed { .. } => "intake proposed".into(),
        Event::ThreadSpawnDenied {
            axis,
            limit,
            observed,
            ..
        } => format!("spawn refused: {axis} at {observed} of {limit}"),
        Event::ReviewSubmitted { decision, .. } => match decision {
            ReviewDecision::RequestChanges => "changes requested".into(),
            ReviewDecision::Approve => "approved".into(),
        },
        Event::ApprovalRequested { .. } => "approval requested".into(),
        Event::WaitTimedOut { reason, .. } => match reason {
            Some(reason) => format!("wait timed out: {reason}"),
            None => "wait timed out".into(),
        },
        Event::ThreadLanded { repo, .. } => format!("landed in {repo}"),
        Event::ThreadResultSet { .. } => "result set".into(),
        _ => return None,
    })
}

/// The oldest unread notification `member_id` got for an event after
/// `after`, skipping one in a thread the caller can no longer read.
async fn next_unread_notification(
    store: &dyn Store,
    auth: &AuthContext,
    member_id: MemberId,
    after: i64,
) -> Result<Option<Notification>, McpError> {
    let mut rows = store
        .list_notifications(member_id, true, NOTIFICATION_SCAN)
        .await?;
    rows.retain(|n| n.source_log_id > after);
    rows.sort_by_key(|n| n.source_log_id);
    for row in rows {
        if !auth.bypass
            && let Some(thread_id) = row.thread_id
            && !maidan_auth::can_access_thread(store, auth, thread_id).await?
        {
            continue;
        }
        return Ok(Some(row));
    }
    Ok(None)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListNotificationsArgs {
    member_id: uuid::Uuid,
    #[serde(default)]
    unread_only: bool,
    #[serde(default)]
    limit: Option<i64>,
}

/// A member's per-recipient notifications, newest first, optionally unread-only
/// — the MCP twin of `GET /members/:id/notifications`. Mirrors the sibling
/// inbox tools (`get_inbox`/`list_mentions`): a member-scoped read, no
/// per-channel filter.
pub(super) async fn list_notifications(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListNotificationsArgs = crate::tools::parse_args(args)?;
    let notes = store
        .list_notifications(MemberId(a.member_id), a.unread_only, clamp_limit(a.limit))
        .await?;
    let mut threads = ThreadCache::default();
    let mut lines = Vec::with_capacity(notes.len());
    for note in notes {
        lines.push(notification_line(store.as_ref(), auth, &mut threads, note).await?);
    }
    Ok(content_json(&lines))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitingArgs {
    member_id: uuid::Uuid,
    /// Seconds an item may wait before it is flagged overdue (default 86400 = 24h).
    #[serde(default)]
    sla_secs: Option<i64>,
}

/// The waiting-on-you inbox: a member's assigned non-terminal threads + the
/// reviews requested from them + the reviews that name no reviewer and fall to
/// them + the workspace's pending approval gates +
/// their unread mentions, oldest-waiting
/// first, each aged against `sla_secs`. `workspace:read`.
pub(super) async fn get_waiting_inbox(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitingArgs = crate::tools::parse_args(args)?;
    let member_id = MemberId(a.member_id);
    let member = store.get_member(member_id).await?;
    let sla = a.sla_secs.filter(|&s| s > 0).unwrap_or(86_400);
    let assigned = store
        .list_assigned_threads(member.workspace_id, member_id)
        .await?;
    // Being named a reviewer does not grant access to a private channel: list
    // only the requests the caller can open (and so can act on).
    let mut reviews = Vec::new();
    for thread in store
        .list_review_requests(member.workspace_id, member_id)
        .await?
    {
        if auth.bypass || maidan_auth::can_access_thread(store.as_ref(), auth, thread.id).await? {
            reviews.push(thread);
        }
    }
    let unassigned = maidan_auth::visible_unassigned_reviews(
        store.as_ref(),
        auth,
        member.workspace_id,
        member_id,
    )
    .await?;
    let gates = store
        .list_pending_approval_gates(member.workspace_id, Some(member_id), 200)
        .await?;
    let all_mentions = store.list_mentions_for_member(member_id, 100).await?;
    let last_read = store.get_inbox_last_read_at(member_id).await?;
    let unread: Vec<_> = all_mentions
        .into_iter()
        .filter(|m| m.created_at > last_read)
        .collect();
    let waits =
        maidan_auth::visible_human_waits(store.as_ref(), auth, member.workspace_id, member_id)
            .await?;
    let inbox = maidan_types::assemble_waiting_inbox(
        &maidan_types::WaitingSources {
            assigned: &assigned,
            review_requests: &reviews,
            unassigned_reviews: &unassigned,
            pending_gates: &gates,
            unread_mentions: &unread,
            blocked: &waits.blocked,
            questions: &waits.questions,
        },
        chrono::Utc::now(),
        sla,
    );
    Ok(content_json(&inbox))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuriedDecisionsArgs {
    member_id: uuid::Uuid,
    /// Only decisions produced after this RFC 3339 instant (default: 7 days ago).
    #[serde(default)]
    since: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagerDigestArgs {
    member_id: uuid::Uuid,
    /// Only unread lifecycle notifications after this RFC 3339 instant.
    #[serde(default)]
    since: Option<chrono::DateTime<chrono::Utc>>,
}

pub(super) async fn get_manager_digest(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ManagerDigestArgs = crate::tools::parse_args(args)?;
    let member_id = MemberId(a.member_id);
    super::requested_member(server.store.as_ref(), auth, member_id).await?;
    if !auth.bypass && auth.token_id.is_none() && auth.member_id != member_id {
        return Err(McpError::Forbidden(
            "a session caller may only read its own manager digest".into(),
        ));
    }
    let since = a
        .since
        .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(7));
    Ok(content_json(
        &server
            .store
            .manager_digest_for_member(member_id, since)
            .await?,
    ))
}

/// A member's buried decisions — task results produced by someone else in a
/// channel/thread the member follows, since `since` (default 7 days ago),
/// newest first. The decisions the digest surfaces, queryable directly.
pub(super) async fn list_buried_decisions(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: BuriedDecisionsArgs = crate::tools::parse_args(args)?;
    let since = a
        .since
        .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(7));
    let limit = a.limit.unwrap_or(50).clamp(1, 200);
    let decisions = store
        .buried_decisions_for_member(MemberId(a.member_id), since, limit)
        .await?;
    Ok(content_json(&decisions))
}

/// A member's notifications collapsed into per-thread groups, newest-activity
/// first — the MCP twin of `GET /members/:id/notifications/grouped`. A busy
/// thread shows as one group instead of flooding the flat list.
pub(super) async fn list_notifications_grouped(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListNotificationsArgs = crate::tools::parse_args(args)?;
    let notes = store
        .list_notifications(MemberId(a.member_id), a.unread_only, clamp_limit(a.limit))
        .await?;
    Ok(content_json(&maidan_types::group_notifications_by_thread(
        &notes,
    )))
}

#[derive(Deserialize)]
struct MemberIdArg {
    member_id: uuid::Uuid,
}

/// A member's unread-notification badge count.
pub(super) async fn get_unread_count(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArg = crate::tools::parse_args(args)?;
    let count = store
        .unread_notification_count(MemberId(a.member_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "count": count })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkNotificationReadArgs {
    member_id: uuid::Uuid,
    notification_id: uuid::Uuid,
}

/// Mark one of a member's notifications read. Recipient-scoped in the store, so
/// `{marked: false}` when the id isn't this member's.
pub(super) async fn mark_notification_read(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MarkNotificationReadArgs = crate::tools::parse_args(args)?;
    let marked = store
        .mark_notification_read(MemberId(a.member_id), NotificationId(a.notification_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "marked": marked })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnoozeNotificationArgs {
    member_id: uuid::Uuid,
    notification_id: uuid::Uuid,
    /// RFC 3339 instant to snooze until.
    until: chrono::DateTime<chrono::Utc>,
}

/// Snooze one of a member's notifications until `until` — it drops out of the
/// inbox + badge until then. Recipient-scoped, so `{snoozed: false}` when the
/// id isn't this member's.
pub(super) async fn snooze_notification(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SnoozeNotificationArgs = crate::tools::parse_args(args)?;
    let snoozed = store
        .snooze_notification(
            MemberId(a.member_id),
            NotificationId(a.notification_id),
            a.until,
        )
        .await?;
    Ok(content_json(&serde_json::json!({ "snoozed": snoozed })))
}

/// Block until `member_id` has an unread notification for an event after
/// `since_log_id` (the log head when omitted), or the timeout lapses. Reads
/// the notification ledger itself rather than the events, so it wakes for
/// every kind the router writes for this member, as owner, follower or the
/// one mentioned, and with `since_log_id` it returns a row written while the
/// caller was not waiting. Returns that row with its digest `title` and
/// `reason`, or `null`.
pub(super) async fn wait_for_notification(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForMentionArgs = crate::tools::parse_args(args)?;
    let member_id = MemberId(a.member_id);
    let store = server.store.as_ref();
    let wait = a
        .timeout_ms
        .unwrap_or(DEFAULT_WAIT_MS)
        .clamp(1, MAX_WAIT_MS);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait as u64);
    // Subscribed before the head is read, so an event in between still wakes
    // the wait early; the poll is what makes it correct without a bus.
    let mut stream = match server.event_bus.as_ref() {
        Some(bus) => Some(
            bus.subscribe(EventFilter::workspace(auth.workspace_id))
                .await
                .map_err(|e| McpError::Internal(e.to_string()))?,
        ),
        None => None,
    };
    let after = match a.since_log_id {
        Some(since) => since,
        None => store.max_event_id().await?,
    };
    loop {
        if let Some(row) = next_unread_notification(store, auth, member_id, after).await? {
            let mut threads = ThreadCache::default();
            let line = notification_line(store, auth, &mut threads, row).await?;
            return Ok(content_json(&line));
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(content_json(&Value::Null));
        }
        let tick = (now + NOTIFICATION_POLL).min(deadline);
        let mut closed = false;
        match stream.as_mut() {
            Some(events) => {
                tokio::select! {
                    item = events.next() => match item {
                        Some(_) => {
                            let settle = tokio::time::Instant::now() + NOTIFICATION_SETTLE;
                            tokio::time::sleep_until(settle.min(deadline)).await;
                        }
                        None => closed = true,
                    },
                    () = tokio::time::sleep_until(tick) => {}
                }
            }
            None => tokio::time::sleep_until(tick).await,
        }
        if closed {
            stream = None;
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetNotificationPrefArgs {
    member_id: uuid::Uuid,
    /// The event kind to (un)mute, snake_case (e.g. `mention_recorded`).
    kind: String,
    muted: bool,
}

/// Set a member's mute preference for an event kind — the MCP twin of `PUT
/// /members/:id/notification-prefs`. The router skips writing notifications of
/// a muted kind for this member.
pub(super) async fn set_notification_pref(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetNotificationPrefArgs = crate::tools::parse_args(args)?;
    let kind = EventKind::parse(&a.kind)
        .ok_or_else(|| McpError::InvalidParams(format!("unknown event kind: {}", a.kind)))?;
    let pref = store
        .set_notification_pref(MemberId(a.member_id), kind, a.muted)
        .await?;
    Ok(content_json(&pref))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListNotificationPrefsArgs {
    member_id: uuid::Uuid,
}

/// A member's notification preferences.
pub(super) async fn list_notification_prefs(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListNotificationPrefsArgs = crate::tools::parse_args(args)?;
    let prefs = store.list_notification_prefs(MemberId(a.member_id)).await?;
    Ok(content_json(&prefs))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDeliveryModeArgs {
    member_id: uuid::Uuid,
    /// `immediate` (per-notification emails) or `digest` (a periodic rollup).
    mode: String,
}

/// Set a member's email delivery mode — the MCP twin of `PUT
/// /members/:id/delivery-mode`. `digest` opts out of per-notification emails in
/// favour of the sweeper's periodic rollup.
pub(super) async fn set_delivery_mode(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetDeliveryModeArgs = crate::tools::parse_args(args)?;
    let mode = EmailDeliveryMode::parse(&a.mode)
        .ok_or_else(|| McpError::InvalidParams(format!("unknown delivery mode: {}", a.mode)))?;
    store.set_delivery_mode(MemberId(a.member_id), mode).await?;
    Ok(content_json(&serde_json::json!({ "mode": mode.as_str() })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetDeliveryModeArgs {
    member_id: uuid::Uuid,
}

/// A member's email delivery mode — `immediate` when never set.
pub(super) async fn get_delivery_mode(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetDeliveryModeArgs = crate::tools::parse_args(args)?;
    let mode = store.get_delivery_mode(MemberId(a.member_id)).await?;
    Ok(content_json(&serde_json::json!({ "mode": mode.as_str() })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetMemberEmailArgs {
    member_id: uuid::Uuid,
    email: String,
}

/// Set a member's delivery email — the MCP twin of `PUT /members/:id/email`.
/// Light `@` sanity check; full validation at the transport.
pub(super) async fn set_member_email(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetMemberEmailArgs = crate::tools::parse_args(args)?;
    let email = a.email.trim();
    if !email.contains('@') || email.len() < 3 {
        return Err(McpError::InvalidParams(
            "email must be a valid address".to_string(),
        ));
    }
    let stored = store.set_member_email(MemberId(a.member_id), email).await?;
    Ok(content_json(&stored))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberEmailArgs {
    member_id: uuid::Uuid,
}

/// A member's delivery email, or `null` if unset.
pub(super) async fn get_member_email(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberEmailArgs = crate::tools::parse_args(args)?;
    let email = store.get_member_email(MemberId(a.member_id)).await?;
    Ok(content_json(&email))
}

/// Clear a member's delivery email — opting out. Returns `{deleted}`.
pub(super) async fn delete_member_email(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberEmailArgs = crate::tools::parse_args(args)?;
    let deleted = store.delete_member_email(MemberId(a.member_id)).await?;
    Ok(content_json(&serde_json::json!({ "deleted": deleted })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowChannelArgs {
    member_id: uuid::Uuid,
    channel_id: uuid::Uuid,
}

/// Follow a channel to be notified of activity there. Channel access is
/// enforced pre-dispatch (the `channel_id` arg). Idempotent.
pub(super) async fn follow_channel(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: FollowChannelArgs = crate::tools::parse_args(args)?;
    store
        .follow_channel(MemberId(a.member_id), ChannelId(a.channel_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "following": true })))
}

/// Unfollow a channel. `removed` is false if not following.
pub(super) async fn unfollow_channel(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: FollowChannelArgs = crate::tools::parse_args(args)?;
    let removed = store
        .unfollow_channel(MemberId(a.member_id), ChannelId(a.channel_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "removed": removed })))
}

/// The channels a member follows.
pub(super) async fn list_channel_follows(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArg = crate::tools::parse_args(args)?;
    Ok(content_json(
        &store.list_channel_follows(MemberId(a.member_id)).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowThreadArgs {
    member_id: uuid::Uuid,
    thread_id: uuid::Uuid,
}

/// Follow a thread to be notified of activity there. Thread access is enforced
/// pre-dispatch (the `thread_id` arg). Idempotent.
pub(super) async fn follow_thread(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: FollowThreadArgs = crate::tools::parse_args(args)?;
    store
        .follow_thread(MemberId(a.member_id), ThreadId(a.thread_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "following": true })))
}

/// Unfollow a thread. `removed` is false if not following.
pub(super) async fn unfollow_thread(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: FollowThreadArgs = crate::tools::parse_args(args)?;
    let removed = store
        .unfollow_thread(MemberId(a.member_id), ThreadId(a.thread_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "removed": removed })))
}

/// The threads a member follows.
pub(super) async fn list_thread_follows(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArg = crate::tools::parse_args(args)?;
    Ok(content_json(
        &store.list_thread_follows(MemberId(a.member_id)).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowMemberArgs {
    member_id: uuid::Uuid,
    followed_member_id: uuid::Uuid,
}

/// Follow another member's work occupancy. Both members must share a
/// workspace; a self-edge is rejected before the schema backstop.
pub(super) async fn follow_member(
    server: &crate::server::McpServer,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: FollowMemberArgs = crate::tools::parse_args(args)?;
    let follower_id = MemberId(a.member_id);
    let followed_id = MemberId(a.followed_member_id);
    if follower_id == followed_id {
        return Err(McpError::InvalidParams(
            "a member cannot follow itself".to_string(),
        ));
    }
    let follower = super::requested_member(server.store.as_ref(), auth, follower_id).await?;
    server
        .store
        .get_member_in(follower.workspace_id, followed_id)
        .await?;
    server.store.follow_member(follower_id, followed_id).await?;
    Ok(content_json(&serde_json::json!({ "following": true })))
}

/// Stop following another member's occupancy.
pub(super) async fn unfollow_member(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: FollowMemberArgs = crate::tools::parse_args(args)?;
    let removed = store
        .unfollow_member(MemberId(a.member_id), MemberId(a.followed_member_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "removed": removed })))
}

/// The member-occupancy subscriptions owned by a member.
pub(super) async fn list_member_follows(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArg = crate::tools::parse_args(args)?;
    Ok(content_json(
        &store.list_member_follows(MemberId(a.member_id)).await?,
    ))
}

/// Live member occupancy, filtered to assigned threads the caller may read.
pub(super) async fn get_member_occupancy(
    server: &crate::server::McpServer,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArg = crate::tools::parse_args(args)?;
    let member_id = MemberId(a.member_id);
    let member = super::requested_member(server.store.as_ref(), auth, member_id).await?;
    let assigned = server
        .store
        .list_assigned_threads(member.workspace_id, member_id)
        .await?;
    let mut visible = Vec::with_capacity(assigned.len());
    for thread in assigned {
        if auth.bypass
            || maidan_auth::can_access_thread(server.store.as_ref(), auth, thread.id).await?
        {
            visible.push(thread);
        }
    }
    Ok(content_json(&maidan_types::MemberOccupancy {
        member_id,
        presence: server.member_presence(member.workspace_id, member_id),
        assigned_threads: visible,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListMembersArgs {
    workspace_id: uuid::Uuid,
}

/// Members of a workspace. Twin of `GET /workspaces/{wid}/members`.
pub(super) async fn list_members(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListMembersArgs = crate::tools::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let members = store.list_members(workspace_id).await?;
    Ok(content_json(&members))
}

/// One member in the caller workspace. Twin of `GET /members/{id}`.
/// Another workspace member is not found, same as an unknown id.
pub(super) async fn get_member(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArg = crate::tools::parse_args(args)?;
    let member = super::requested_member(store.as_ref(), auth, MemberId(a.member_id)).await?;
    Ok(content_json(&member))
}

/// Mark every notification for this member read. Twin of
/// `POST /members/{id}/notifications/read-all`, named beside `mark_notification_read`.
pub(super) async fn mark_all_notifications_read(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArg = crate::tools::parse_args(args)?;
    let cleared = store
        .mark_all_notifications_read(MemberId(a.member_id))
        .await? as i64;
    Ok(content_json(&serde_json::json!({ "cleared": cleared })))
}
