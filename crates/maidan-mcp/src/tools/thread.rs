//! Thread listing, assignment, dependency, and result tool handlers.

use std::sync::Arc;

use chrono::Utc;
use futures::StreamExt;
use maidan_auth::AuthContext;
use maidan_router::resolve_thread_context;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

/// `wait_for_ready` long-poll window default + ceiling (Cluster 223), mirroring
/// `wait_for_mention`.
const DEFAULT_WAIT_MS: i64 = 30_000;
const MAX_WAIT_MS: i64 = 300_000;
/// Page size for the lookback replay over the durable event log (Cluster 354).
const LOOKBACK_BATCH: i64 = 256;

/// Replay the durable event log for the earliest event of one of `kinds` with
/// `log_id > since` (Cluster 354 lookback), optionally pinned to `channel_id`
/// and/or `thread_id`, in the caller's workspace. When `rbac_thread` is set, an
/// event in a thread the caller can't access is skipped (not revealed) — the
/// live path's rule. Returns the deserialized `Event`, or `None` if the log has
/// no such event. Filters on the `StoredEvent` columns before deserializing.
async fn lookback_event(
    store: &dyn Store,
    auth: &AuthContext,
    kinds: &std::collections::HashSet<EventKind>,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
    since: i64,
    rbac_thread: bool,
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
            if let Some(cid) = channel_id {
                if stored.channel_id != Some(cid) {
                    continue;
                }
            }
            if let Some(tid) = thread_id {
                if stored.thread_id != Some(tid) {
                    continue;
                }
            }
            let event: Event = serde_json::from_value(stored.payload.clone())
                .map_err(|e| McpError::Internal(e.to_string()))?;
            if rbac_thread && !auth.bypass {
                if let Some(tid) = event.thread_id() {
                    if !maidan_auth::can_access_thread(store, auth, tid).await? {
                        continue;
                    }
                }
            }
            return Ok(Some(event));
        }
        if drained {
            return Ok(None);
        }
    }
}

#[derive(Deserialize)]
struct ListThreadsArgs {
    channel_id: uuid::Uuid,
    /// Max threads to return (default 100, clamped 1..=500) — Cluster 343.
    #[serde(default)]
    limit: Option<i64>,
    /// Exclusive keyset cursor: the prior page's last thread id.
    #[serde(default)]
    cursor: Option<uuid::Uuid>,
}

pub(super) async fn list_threads(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListThreadsArgs = serde_json::from_value(args.clone())?;
    // Cluster 343: keyset-paginated (was unbounded). Default 100, clamp 1..=500.
    let limit = a.limit.unwrap_or(100).clamp(1, 500);
    let after = a.cursor.map(ThreadId);
    let threads = store
        .page_threads_for_channel(ChannelId(a.channel_id), after, limit)
        .await?;
    Ok(content_json(&threads))
}

#[derive(Deserialize)]
struct ThreadIdArg {
    thread_id: uuid::Uuid,
}

/// A parent thread's child threads, collapsed with a message count each
/// (Cluster 356, F2, the MCP twin of `GET /threads/:id/children`). Thread access
/// is enforced pre-dispatch.
pub(super) async fn list_child_threads(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadIdArg = serde_json::from_value(args.clone())?;
    let summaries = store.child_thread_summaries(ThreadId(a.thread_id)).await?;
    Ok(content_json(&summaries))
}

#[derive(Deserialize)]
struct RecentThreadsArgs {
    channel_id: uuid::Uuid,
    /// Max threads to return (default 50, clamped 1..=200).
    #[serde(default)]
    limit: Option<i64>,
}

/// A channel's threads ordered by last activity — most-recently bumped first
/// (Cluster 356, F7, the MCP twin of `GET /channels/:cid/recent-threads`).
/// Channel access is enforced pre-dispatch.
pub(super) async fn list_recently_active_threads(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RecentThreadsArgs = serde_json::from_value(args.clone())?;
    let limit = a.limit.unwrap_or(50).clamp(1, 200);
    let threads = store
        .list_recently_active_threads(ChannelId(a.channel_id), limit)
        .await?;
    Ok(content_json(&threads))
}

/// Mute a thread for the caller (Cluster 356, F7, the MCP twin of
/// `POST /threads/:id/mute`). The notification router then suppresses this
/// thread's notifications for the caller. Thread access is enforced pre-dispatch.
pub(super) async fn mute_thread(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadIdArg = serde_json::from_value(args.clone())?;
    store
        .mute_thread(auth.member_id, ThreadId(a.thread_id))
        .await?;
    Ok(content_json(&json!({ "muted": true })))
}

/// Unmute a thread for the caller (Cluster 356, F7, the MCP twin of
/// `DELETE /threads/:id/mute`). `{unmuted}` is `false` when it was not muted.
pub(super) async fn unmute_thread(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadIdArg = serde_json::from_value(args.clone())?;
    let unmuted = store
        .unmute_thread(auth.member_id, ThreadId(a.thread_id))
        .await?;
    Ok(content_json(&json!({ "unmuted": unmuted })))
}

#[derive(Deserialize)]
struct ToolTranscriptArgs {
    thread_id: uuid::Uuid,
    /// Max messages to scan (default 200, clamped 1..=500).
    #[serde(default)]
    limit: Option<i64>,
}

/// A thread's tool-call transcript (Cluster 197): every `ToolUse` block across
/// the thread's messages, each correlated with its `ToolResult` by id. A
/// token-lean projection — `Text`/`Code` blocks and `body` are dropped. Channel
/// access is enforced pre-dispatch (the `thread_id` arg).
pub(super) async fn get_tool_transcript(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ToolTranscriptArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let limit = a.limit.unwrap_or(200).clamp(1, 500);
    let messages = store.list_messages(thread_id, limit).await?;
    Ok(content_json(&tool_transcript(thread_id, &messages)))
}

#[derive(Deserialize)]
struct AssignThreadArgs {
    thread_id: uuid::Uuid,
    actor_id: uuid::Uuid,
    assignee_id: uuid::Uuid,
    /// Optional handoff note for the assignee (Cluster 195).
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
struct ClaimThreadArgs {
    thread_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

#[derive(Deserialize)]
struct UnassignThreadArgs {
    thread_id: uuid::Uuid,
    actor_id: uuid::Uuid,
}

/// Emit a `ThreadAssignmentChanged` event for an assignment mutation
/// (Cluster 171). No-op when the bus is unconfigured.
async fn publish_assignment(
    server: &crate::server::McpServer,
    thread: &Thread,
    actor_id: MemberId,
    previous_assignee_id: Option<MemberId>,
    note: Option<String>,
) -> Result<(), McpError> {
    if server.event_bus.is_none() {
        return Ok(());
    }
    let ctx = resolve_thread_context(server.store.as_ref(), thread.id)
        .await
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    server
        .publish_event(Event::ThreadAssignmentChanged {
            occurred_at: Utc::now(),
            workspace_id: ctx.workspace_id,
            channel_id: ctx.channel_id,
            thread_id: thread.id,
            actor_id,
            previous_assignee_id,
            assignee_id: thread.assignee_id,
            note,
            thread: thread.clone(),
        })
        .await;
    Ok(())
}

pub(super) async fn assign_thread(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AssignThreadArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let previous = server.store.get_thread(thread_id).await?.assignee_id;
    let thread = server
        .store
        .assign_thread(thread_id, MemberId(a.assignee_id))
        .await?;
    publish_assignment(server, &thread, MemberId(a.actor_id), previous, a.note).await?;
    Ok(content_json(&thread))
}

/// Whether `member` is at (or over) their workspace's WIP limit (Cluster 362,
/// G11) — the MCP twin of `routes::at_wip_limit`. `false` when unset (unlimited).
async fn at_wip_limit(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<bool, McpError> {
    match store.get_wip_limit(workspace_id).await? {
        Some(limit) => Ok(store.count_live_claims(member_id).await? >= limit),
        None => Ok(false),
    }
}

pub(super) async fn claim_thread(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ClaimThreadArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let member_id = MemberId(a.member_id);
    // Unclaimable (Cluster 363, G3): a parked thread refuses an explicit claim,
    // just as `claim_next` skips it (the REST 409 analogue).
    if let Some(u) = server.store.get_thread_unclaimable(thread_id).await? {
        return Err(McpError::InvalidParams(format!(
            "thread is parked (unclaimable): {}",
            u.reason
        )));
    }
    // WIP limit (Cluster 362, G11): refuse a NEW claim past the cap (the REST
    // 409 analogue); a re-claim of a thread the member already holds is exempt.
    let thread = server.store.get_thread(thread_id).await?;
    if thread.assignee_id != Some(member_id) {
        let channel = server.store.get_channel(thread.channel_id).await?;
        if at_wip_limit(server.store.as_ref(), channel.workspace_id, member_id).await? {
            return Err(McpError::InvalidParams(
                "member is at the workspace WIP limit; release or finish a claim first".into(),
            ));
        }
    }
    let result = server.store.claim_thread(thread_id, member_id).await?;
    if result.claimed {
        publish_assignment(server, &result.thread, member_id, None, None).await?;
    }
    Ok(content_json(&result))
}

pub(super) async fn unassign_thread(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UnassignThreadArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let previous = server.store.get_thread(thread_id).await?.assignee_id;
    let thread = server.store.unassign_thread(thread_id).await?;
    publish_assignment(server, &thread, MemberId(a.actor_id), previous, None).await?;
    Ok(content_json(&thread))
}

#[derive(Deserialize)]
struct MarkUnclaimableArgs {
    thread_id: uuid::Uuid,
    reason: String,
}

/// Park a thread from dispatch (Cluster 363, G3): `claim_next` skips it and an
/// explicit `claim` is refused, until cleared. `thread:transition`; thread access
/// is enforced pre-dispatch. Empty reason → InvalidParams.
pub(super) async fn mark_unclaimable(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MarkUnclaimableArgs = serde_json::from_value(args.clone())?;
    let reason = a.reason.trim();
    if reason.is_empty() {
        return Err(McpError::InvalidParams("reason must not be empty".into()));
    }
    let marked = store
        .mark_thread_unclaimable(ThreadId(a.thread_id), reason, auth.member_id)
        .await?;
    Ok(content_json(&marked))
}

/// Un-park a thread (Cluster 363) — it becomes claimable again. `{cleared}` is
/// `false` when it was not parked. `thread:transition`; thread access enforced.
pub(super) async fn mark_claimable(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadIdArg = serde_json::from_value(args.clone())?;
    let cleared = store.mark_thread_claimable(ThreadId(a.thread_id)).await?;
    Ok(content_json(&json!({ "cleared": cleared })))
}

#[derive(Deserialize)]
struct ChannelIdArg {
    channel_id: uuid::Uuid,
}

/// The parked (unclaimable) threads in a channel (Cluster 363), newest first.
/// `workspace:read`; channel access enforced pre-dispatch.
pub(super) async fn list_unclaimable(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ChannelIdArg = serde_json::from_value(args.clone())?;
    let parked = store
        .list_unclaimable_threads(ChannelId(a.channel_id))
        .await?;
    Ok(content_json(&parked))
}

#[derive(Deserialize)]
struct SetWipLimitArgs {
    #[serde(default)]
    limit: Option<i64>,
}

/// Set (or clear) the caller's workspace WIP limit (Cluster 362, G11): the max
/// concurrent live claims per member. `limit >= 0` caps (0 freezes); omit/null
/// clears it (unlimited). `workspace:write`.
pub(super) async fn set_wip_limit(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetWipLimitArgs = serde_json::from_value(args.clone())?;
    if let Some(limit) = a.limit {
        if limit < 0 {
            return Err(McpError::InvalidParams("limit must be >= 0".into()));
        }
    }
    store.set_wip_limit(auth.workspace_id, a.limit).await?;
    Ok(content_json(&json!({ "limit": a.limit })))
}

/// The caller's workspace WIP limit, or null (unlimited) (Cluster 362).
/// `workspace:read`.
pub(super) async fn get_wip_limit(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let limit = store.get_wip_limit(auth.workspace_id).await?;
    Ok(content_json(&json!({ "limit": limit })))
}

#[derive(Deserialize)]
struct MemberWipArgs {
    member_id: uuid::Uuid,
}

/// A member's live-claim count vs the workspace WIP limit (Cluster 362).
/// Member-scoped (the member's own workspace). `workspace:read`.
pub(super) async fn get_member_wip(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberWipArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    let member = store.get_member(member_id).await?;
    let live_claims = store.count_live_claims(member_id).await?;
    let limit = store.get_wip_limit(member.workspace_id).await?;
    Ok(content_json(
        &json!({ "live_claims": live_claims, "limit": limit }),
    ))
}

#[derive(Deserialize)]
struct ListAssignedThreadsArgs {
    member_id: uuid::Uuid,
}

/// A member's assigned-thread queue (Cluster 191). A member-scoped aggregate
/// read: the pre-dispatch channel gate can't cover a `member_id` arg, so this
/// filters the result to threads the caller can access (like `search_messages`).
pub(super) async fn list_assigned_threads(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListAssignedThreadsArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    let member = store.get_member(member_id).await?;
    let threads = store
        .list_assigned_threads(member.workspace_id, member_id)
        .await?;
    if auth.bypass {
        return Ok(content_json(&threads));
    }
    let mut visible = Vec::with_capacity(threads.len());
    for t in threads {
        if maidan_auth::can_access_thread(store.as_ref(), auth, t.id).await? {
            visible.push(t);
        }
    }
    Ok(content_json(&visible))
}

#[derive(Deserialize)]
struct ClaimNextThreadArgs {
    channel_id: uuid::Uuid,
    member_id: uuid::Uuid,
    /// Optional lease deadline in seconds; the claim is reclaimable after it
    /// lapses (Cluster 192). Omit for a durable claim.
    #[serde(default)]
    lease_secs: Option<i64>,
}

/// Atomically claim the oldest claimable thread in a channel (Cluster 191/192).
/// Channel access is enforced pre-dispatch (the `channel_id` arg). Returns the
/// claimed thread, or `null` when there is no claimable work.
pub(super) async fn claim_next_thread(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ClaimNextThreadArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    // WIP limit (Cluster 362, G11): a capped member is dispatched nothing (null),
    // the same shape as an empty queue.
    let channel = server.store.get_channel(ChannelId(a.channel_id)).await?;
    if at_wip_limit(server.store.as_ref(), channel.workspace_id, member_id).await? {
        return Ok(content_json(&Value::Null));
    }
    let claimed = server
        .store
        .claim_next_thread(ChannelId(a.channel_id), member_id, a.lease_secs)
        .await?;
    if let Some(thread) = &claimed {
        publish_assignment(server, thread, member_id, None, None).await?;
    }
    Ok(content_json(&claimed))
}

#[derive(Deserialize)]
struct RenewClaimArgs {
    thread_id: uuid::Uuid,
    member_id: uuid::Uuid,
    claim_lease_id: uuid::Uuid,
    lease_secs: i64,
}

/// Extend a claimed thread's lease (heartbeat), only for the current assignee
/// holding the matching fencing token (Cluster 192 / 351). `claim_lease_id` is
/// the value from the claiming response's `Thread.claim_lease_id`; a stale
/// holder presents an outdated token and is rejected. Thread access is enforced
/// pre-dispatch (the `thread_id` arg).
pub(super) async fn renew_claim(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RenewClaimArgs = serde_json::from_value(args.clone())?;
    let thread = server
        .store
        .renew_claim(
            ThreadId(a.thread_id),
            MemberId(a.member_id),
            ClaimLeaseId(a.claim_lease_id),
            a.lease_secs,
        )
        .await?;
    Ok(content_json(&thread))
}

#[derive(Deserialize)]
struct AcknowledgeClaimArgs {
    thread_id: uuid::Uuid,
    member_id: uuid::Uuid,
    claim_lease_id: uuid::Uuid,
}

/// Acknowledge a claim and start the working clock (Cluster 351), for the current
/// holder presenting the matching fencing token. Idempotent (the first start time
/// wins). Thread access is enforced pre-dispatch (the `thread_id` arg).
pub(super) async fn acknowledge_claim(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AcknowledgeClaimArgs = serde_json::from_value(args.clone())?;
    let thread = server
        .store
        .acknowledge_claim(
            ThreadId(a.thread_id),
            MemberId(a.member_id),
            ClaimLeaseId(a.claim_lease_id),
        )
        .await?;
    Ok(content_json(&thread))
}

#[derive(Deserialize)]
struct ReleaseClaimArgs {
    thread_id: uuid::Uuid,
    member_id: uuid::Uuid,
    claim_lease_id: uuid::Uuid,
}

/// Release a claim (graceful handoff, Cluster 351): the current holder returns the
/// thread to the queue by presenting its fencing token, instead of waiting for the
/// lease to lapse. Thread access is enforced pre-dispatch. Emits
/// `ThreadAssignmentChanged`.
pub(super) async fn release_claim(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ReleaseClaimArgs = serde_json::from_value(args.clone())?;
    let member = MemberId(a.member_id);
    let thread = server
        .store
        .release_claim(
            ThreadId(a.thread_id),
            member,
            ClaimLeaseId(a.claim_lease_id),
        )
        .await?;
    publish_assignment(server, &thread, member, Some(member), None).await?;
    Ok(content_json(&thread))
}

#[derive(Deserialize)]
struct AddThreadDependencyArgs {
    thread_id: uuid::Uuid,
    depends_on_thread_id: uuid::Uuid,
}

/// Add a task-dependency edge (Cluster 220): the path/`thread_id` task depends on
/// `depends_on_thread_id`. The primary `thread_id`'s channel access is enforced
/// pre-dispatch; the `depends_on` thread is checked here (the gate covers only one
/// id), plus a same-workspace guard. Idempotent; a self-dependency is rejected.
pub(super) async fn add_thread_dependency(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AddThreadDependencyArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let depends_on = ThreadId(a.depends_on_thread_id);
    let ctx = resolve_thread_context(store.as_ref(), thread_id)
        .await
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let dep_ctx = resolve_thread_context(store.as_ref(), depends_on)
        .await
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    if dep_ctx.workspace_id != ctx.workspace_id {
        return Err(McpError::InvalidParams(
            "dependency thread is in a different workspace".into(),
        ));
    }
    if !auth.bypass {
        maidan_auth::ensure_thread_access(store.as_ref(), auth, depends_on).await?;
    }
    store.add_thread_dependency(thread_id, depends_on).await?;
    Ok(content_json(&json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ThreadDepsArgs {
    thread_id: uuid::Uuid,
}

/// A task's dependencies + whether it is ready (all deps terminal) — Cluster 220.
/// Channel access is enforced pre-dispatch (the `thread_id` arg).
pub(super) async fn list_thread_dependencies(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadDepsArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let dependencies = store.list_thread_dependencies(thread_id).await?;
    let ready = store.thread_dependencies_satisfied(thread_id).await?;
    Ok(content_json(
        &json!({ "dependencies": dependencies, "ready": ready }),
    ))
}

#[derive(Deserialize)]
struct QueueDepthArgs {
    channel_id: uuid::Uuid,
}

/// A channel's task-queue depth (Cluster 225): `{open, ready, assigned, blocked}`
/// counts of its open task threads — the MCP twin of `GET /channels/:cid/queue-depth`
/// (Cluster 224). Channel access is enforced pre-dispatch (the `channel_id` arg).
pub(super) async fn get_queue_depth(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: QueueDepthArgs = serde_json::from_value(args.clone())?;
    let depth = store.channel_queue_depth(ChannelId(a.channel_id)).await?;
    Ok(content_json(&depth))
}

/// A channel's occupancy (Cluster 351): `{open, queued, claimed, working, blocked}`
/// — the two-clocks refinement of `get_queue_depth`, splitting held work into
/// `claimed` (not yet acknowledged) and `working`. The MCP twin of
/// `GET /channels/:cid/occupancy`. Channel access is enforced pre-dispatch.
pub(super) async fn get_channel_occupancy(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: QueueDepthArgs = serde_json::from_value(args.clone())?;
    let occupancy = store.channel_occupancy(ChannelId(a.channel_id)).await?;
    Ok(content_json(&occupancy))
}

#[derive(Deserialize)]
struct SetThreadResultArgs {
    thread_id: uuid::Uuid,
    result: Value,
}

/// Attach a task's structured result (Cluster 236, the MCP twin of
/// `PUT /threads/:id/result`). Upserts one result per thread (`produced_by` is
/// the caller) and publishes a `ThreadResultSet` event so waiters
/// (`wait_for_result`) wake. Thread access is enforced pre-dispatch (the
/// `thread_id` arg).
pub(super) async fn set_thread_result(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetThreadResultArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let result = server
        .store
        .set_thread_result(thread_id, auth.member_id, &a.result)
        .await?;
    if server.event_bus.is_some() {
        let ctx = resolve_thread_context(server.store.as_ref(), thread_id)
            .await
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        server
            .publish_event(Event::ThreadResultSet {
                occurred_at: Utc::now(),
                workspace_id: ctx.workspace_id,
                channel_id: ctx.channel_id,
                thread_id,
                produced_by: auth.member_id,
            })
            .await;
    }
    Ok(content_json(&result))
}

#[derive(Deserialize)]
struct GetThreadResultArgs {
    thread_id: uuid::Uuid,
}

/// Read a task's structured result, or `null` if none has been produced
/// (Cluster 236, the MCP twin of `GET /threads/:id/result`). Thread access is
/// enforced pre-dispatch.
pub(super) async fn get_thread_result(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetThreadResultArgs = serde_json::from_value(args.clone())?;
    let result = store.get_thread_result(ThreadId(a.thread_id)).await?;
    Ok(content_json(&result))
}

#[derive(Deserialize)]
struct SetThreadOwnerArgs {
    thread_id: uuid::Uuid,
    /// The owner to set; omit (or null) to clear the owner (Cluster 355, W1).
    #[serde(default)]
    owner_id: Option<uuid::Uuid>,
}

/// Set (or clear, by omitting `owner_id`) a thread's durable owner (Cluster 355,
/// W1, the MCP twin of `PUT`/`DELETE /threads/:id/owner`). Thread access is
/// enforced pre-dispatch. Once an owner is set, the claimer can no longer land
/// its own work (separation of duties, enforced in the FSM transition).
pub(super) async fn set_thread_owner(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetThreadOwnerArgs = serde_json::from_value(args.clone())?;
    let thread = store
        .set_thread_owner(ThreadId(a.thread_id), a.owner_id.map(MemberId))
        .await?;
    Ok(content_json(&thread))
}

#[derive(Deserialize)]
struct RenameThreadArgs {
    thread_id: uuid::Uuid,
    title: String,
}

/// Rename a thread (Cluster 356, F1, the MCP twin of `PUT /threads/:id/title`).
/// Rejects a blank title. Thread access is enforced pre-dispatch.
pub(super) async fn rename_thread(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: RenameThreadArgs = serde_json::from_value(args.clone())?;
    let title = a.title.trim();
    if title.is_empty() {
        return Err(McpError::InvalidParams("title must not be empty".into()));
    }
    let thread = store
        .set_thread_title(ThreadId(a.thread_id), Some(title.to_string()))
        .await?;
    Ok(content_json(&thread))
}

#[derive(Deserialize)]
struct SetThreadSteerArgs {
    thread_id: uuid::Uuid,
    steer: String,
}

/// Set (upsert) a thread's persisted steer (Cluster 355, W1, the MCP twin of
/// `PUT /threads/:id/steer`). `steered_by` is the caller. Thread access is
/// enforced pre-dispatch.
pub(super) async fn set_thread_steer(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetThreadSteerArgs = serde_json::from_value(args.clone())?;
    let steer = server
        .store
        .set_thread_steer(ThreadId(a.thread_id), auth.member_id, &a.steer)
        .await?;
    Ok(content_json(&steer))
}

#[derive(Deserialize)]
struct GetThreadSteerArgs {
    thread_id: uuid::Uuid,
}

/// Read a thread's current steer, or `null` if none is set (Cluster 355, W1, the
/// MCP twin of `GET /threads/:id/steer`). A resuming/newly-assigned agent reads
/// this to follow the current steer. Thread access is enforced pre-dispatch.
pub(super) async fn get_thread_steer(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetThreadSteerArgs = serde_json::from_value(args.clone())?;
    let steer = store.get_thread_steer(ThreadId(a.thread_id)).await?;
    Ok(content_json(&steer))
}

#[derive(Deserialize)]
struct WaitForResultArgs {
    thread_id: uuid::Uuid,
    /// Long-poll window in milliseconds (default 30 000, clamped 1 000–300 000).
    #[serde(default)]
    timeout_ms: Option<i64>,
    /// Lookback anchor (Cluster 354): the caller's high-water `log_id`. When set,
    /// replay the log for a `ThreadResultSet` on this thread with `log_id >
    /// since_log_id` before parking live — so a result produced in the gap before
    /// this call subscribes is not missed.
    #[serde(default)]
    since_log_id: Option<i64>,
}

/// Block until a task's structured result is produced — a `ThreadResultSet`
/// event (Cluster 235) for `thread_id` — or the timeout lapses (Cluster 236).
/// Returns the `ThreadResult` (the payload, fetched after the signal) or `null`
/// on timeout. The coordination wait for the "spawn sub-tasks, wait, aggregate"
/// pattern; the `wait_for_ready` analogue. Thread access is enforced
/// pre-dispatch. **Live** primitive: it only sees results produced *after* it
/// subscribes, so read the current result with `get_thread_result` first (the
/// `GET /mcp/stream` SSE transport, `kinds=thread_result_set`, is the resumable
/// alternative when a missed signal is unacceptable).
pub(super) async fn wait_for_result(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForResultArgs = serde_json::from_value(args.clone())?;
    let Some(bus) = server.event_bus.as_ref() else {
        return Err(McpError::InvalidParams(
            "wait_for_result requires an event bus".into(),
        ));
    };
    let wait = a
        .timeout_ms
        .unwrap_or(DEFAULT_WAIT_MS)
        .clamp(1, MAX_WAIT_MS);
    let thread_id = ThreadId(a.thread_id);

    let filter = EventFilter {
        workspace_id: Some(auth.workspace_id),
        thread_id: Some(thread_id),
        kinds: Some(std::collections::HashSet::from([
            EventKind::ThreadResultSet,
        ])),
        ..EventFilter::default()
    };
    let mut stream = bus
        .subscribe(filter)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;

    // Access to `thread_id` is enforced by the pre-dispatch gate; the filter pins
    // the thread, so any event that arrives is the one we're waiting on.
    let store = server.store.as_ref();

    // Lookback (Cluster 354): a result set in the gap before this subscribe is
    // still caught. No RBAC re-check — the pre-dispatch gate already cleared the
    // thread. Subscribe-before-lookback keeps it gapless.
    if let Some(since) = a.since_log_id {
        let kinds = std::collections::HashSet::from([EventKind::ThreadResultSet]);
        if lookback_event(store, auth, &kinds, None, Some(thread_id), since, false)
            .await?
            .is_some()
        {
            let result = store.get_thread_result(thread_id).await?;
            return Ok(content_json(&result));
        }
    }

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(wait as u64);
    loop {
        let item = match tokio::time::timeout_at(deadline, stream.next()).await {
            // Timed out or the bus closed → no result produced in the window.
            Err(_) | Ok(None) => return Ok(content_json(&Value::Null)),
            Ok(Some(item)) => item,
        };
        // A lag marker means the buffer overflowed; keep waiting (same deadline).
        let maidan_bus::BusItem::Event(_) = item else {
            continue;
        };
        let result = store.get_thread_result(thread_id).await?;
        return Ok(content_json(&result));
    }
}

#[derive(Deserialize)]
struct DependencyResultsArgs {
    thread_id: uuid::Uuid,
}

/// Gather the structured results of a parent task's dependencies (Cluster 236) —
/// the "spawn sub-tasks, wait, aggregate their outputs" read. For each
/// dependency edge of `thread_id`, returns `{thread_id, result}` (result `null`
/// if that dependency hasn't produced one yet), skipping dependencies in
/// channels the caller can't access. The parent's access is enforced
/// pre-dispatch; the dependencies (which may live in other channels) are
/// filtered here, like `list_assigned_threads`.
pub(super) async fn get_dependency_results(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: DependencyResultsArgs = serde_json::from_value(args.clone())?;
    let deps = store
        .list_thread_dependencies(ThreadId(a.thread_id))
        .await?;
    let mut out = Vec::with_capacity(deps.len());
    for dep in deps {
        let dep_id = dep.depends_on_thread_id;
        if !auth.bypass && !maidan_auth::can_access_thread(store.as_ref(), auth, dep_id).await? {
            continue;
        }
        // Project to the raw payload (or null) — the parent wants each
        // dependency's output, not the provenance envelope. `null` marks a
        // dependency that hasn't produced a result yet.
        let result = store.get_thread_result(dep_id).await?.map(|r| r.result);
        out.push(json!({ "thread_id": dep_id, "result": result }));
    }
    Ok(content_json(&json!({ "dependencies": out })))
}

#[derive(Deserialize)]
struct WaitForReadyArgs {
    /// Optional channel to scope readiness to; omit to await any accessible ready
    /// thread in the caller's workspace.
    #[serde(default)]
    channel_id: Option<uuid::Uuid>,
    /// Long-poll window in milliseconds (default 30 000, clamped 1 000–300 000).
    #[serde(default)]
    timeout_ms: Option<i64>,
    /// Lookback anchor (Cluster 354): the caller's high-water `log_id`. When set,
    /// replay the log for a `ThreadReady` with `log_id > since_log_id` (in scope,
    /// RBAC-filtered) before parking live.
    #[serde(default)]
    since_log_id: Option<i64>,
}

/// Block until a task becomes ready — its last blocking dependency reached a
/// terminal state, emitting `ThreadReady` (Cluster 222) — or the timeout lapses
/// (Cluster 223). Scoped to `channel_id` when given, else any thread in the
/// caller's workspace they can access; returns the `ThreadReady` event or `null`
/// on timeout. This is the `wait_for_mention` analogue for the DAG. **Live**
/// primitive: it only sees readiness signalled *after* it subscribes, so pick up
/// already-ready work first with `claim_next_thread` / `list_assigned_threads`
/// (the `GET /mcp/stream` SSE transport, `kinds=thread_ready`, is the resumable
/// alternative when a missed signal is unacceptable).
pub(super) async fn wait_for_ready(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForReadyArgs = serde_json::from_value(args.clone())?;
    let Some(bus) = server.event_bus.as_ref() else {
        return Err(McpError::InvalidParams(
            "wait_for_ready requires an event bus".into(),
        ));
    };
    let wait = a
        .timeout_ms
        .unwrap_or(DEFAULT_WAIT_MS)
        .clamp(1, MAX_WAIT_MS);

    let filter = EventFilter {
        workspace_id: Some(auth.workspace_id),
        channel_id: a.channel_id.map(ChannelId),
        kinds: Some(std::collections::HashSet::from([EventKind::ThreadReady])),
        ..EventFilter::default()
    };
    let mut stream = bus
        .subscribe(filter)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;

    let store = server.store.as_ref();

    // Lookback (Cluster 354): a readiness signalled in the gap before this
    // subscribe is still caught, RBAC-filtered like the live path.
    if let Some(since) = a.since_log_id {
        let kinds = std::collections::HashSet::from([EventKind::ThreadReady]);
        if let Some(event) = lookback_event(
            store,
            auth,
            &kinds,
            a.channel_id.map(ChannelId),
            None,
            since,
            true,
        )
        .await?
        {
            return Ok(content_json(&event));
        }
    }

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(wait as u64);
    loop {
        let item = match tokio::time::timeout_at(deadline, stream.next()).await {
            // Timed out or the bus closed → no task became ready in the window.
            Err(_) | Ok(None) => return Ok(content_json(&Value::Null)),
            Ok(Some(item)) => item,
        };
        // A lag marker means the buffer overflowed; keep waiting (same deadline).
        let maidan_bus::BusItem::Event(envelope) = item else {
            continue;
        };
        // Don't reveal readiness of a thread the caller can't access.
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

#[derive(Deserialize)]
struct WaitForClaimExpiredArgs {
    /// Optional channel to scope to; omit to await any accessible expiry in the
    /// caller's workspace.
    #[serde(default)]
    channel_id: Option<uuid::Uuid>,
    /// Long-poll window in milliseconds (default 30 000, clamped 1 000–300 000).
    #[serde(default)]
    timeout_ms: Option<i64>,
    /// Lookback anchor (Cluster 354): the caller's high-water `log_id`. When set,
    /// replay the log for a `ClaimExpired` with `log_id > since_log_id` (in scope,
    /// RBAC-filtered) before parking live.
    #[serde(default)]
    since_log_id: Option<i64>,
}

/// Block until a claim's lease lapses and its thread is reclaimed, emitting
/// `ClaimExpired` (Cluster 351) — a supervisor's "an agent died" signal, so it
/// needn't poll the occupancy view. Scoped to `channel_id` when given, else any
/// thread in the caller's workspace they can access; returns the `ClaimExpired`
/// event (its `member_id` is the dead holder) or `null` on timeout. **Live**
/// primitive (only sees expiries reclaimed *after* it subscribes); the
/// `GET /mcp/stream` SSE transport, `kinds=claim_expired`, is the resumable
/// alternative. A lease that expires but is never reclaimed emits nothing.
pub(super) async fn wait_for_claim_expired(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForClaimExpiredArgs = serde_json::from_value(args.clone())?;
    let Some(bus) = server.event_bus.as_ref() else {
        return Err(McpError::InvalidParams(
            "wait_for_claim_expired requires an event bus".into(),
        ));
    };
    let wait = a
        .timeout_ms
        .unwrap_or(DEFAULT_WAIT_MS)
        .clamp(1, MAX_WAIT_MS);

    let filter = EventFilter {
        workspace_id: Some(auth.workspace_id),
        channel_id: a.channel_id.map(ChannelId),
        kinds: Some(std::collections::HashSet::from([EventKind::ClaimExpired])),
        ..EventFilter::default()
    };
    let mut stream = bus
        .subscribe(filter)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;

    let store = server.store.as_ref();

    // Lookback (Cluster 354): an expiry reclaimed in the gap before this
    // subscribe is still caught, RBAC-filtered like the live path.
    if let Some(since) = a.since_log_id {
        let kinds = std::collections::HashSet::from([EventKind::ClaimExpired]);
        if let Some(event) = lookback_event(
            store,
            auth,
            &kinds,
            a.channel_id.map(ChannelId),
            None,
            since,
            true,
        )
        .await?
        {
            return Ok(content_json(&event));
        }
    }

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(wait as u64);
    loop {
        let item = match tokio::time::timeout_at(deadline, stream.next()).await {
            Err(_) | Ok(None) => return Ok(content_json(&Value::Null)),
            Ok(Some(item)) => item,
        };
        let maidan_bus::BusItem::Event(envelope) = item else {
            continue;
        };
        // Don't reveal an expiry in a thread the caller can't access.
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

#[derive(serde::Deserialize)]
struct WaitForLandedArgs {
    /// Optional thread to scope to — wait for *this* thread's PR to land. Omit to
    /// await any accessible land in the caller's workspace (or channel).
    #[serde(default)]
    thread_id: Option<uuid::Uuid>,
    /// Optional channel to scope to; omit to await any accessible land in the
    /// caller's workspace.
    #[serde(default)]
    channel_id: Option<uuid::Uuid>,
    /// Long-poll window in milliseconds (default 30 000, clamped 1 000–300 000).
    #[serde(default)]
    timeout_ms: Option<i64>,
    /// Lookback anchor (Cluster 354): the caller's high-water `log_id`. When set,
    /// replay the log for a `ThreadLanded` with `log_id > since_log_id` (in scope,
    /// RBAC-filtered) before parking live.
    #[serde(default)]
    since_log_id: Option<i64>,
}

/// `wait_for_landed` — block until a thread's linked GitHub PR lands (Cluster 361,
/// G-dev-7): subscribe to `ThreadLanded` and return the fact, so an agent (or a
/// reviewer awaiting a merge) needn't poll. Scoped to `thread_id` and/or
/// `channel_id` when given, else any accessible land in the caller's workspace;
/// returns the `ThreadLanded` event or `null` on timeout. **Live** primitive (only
/// sees lands emitted *after* it subscribes); the `GET /mcp/stream` SSE transport,
/// `kinds=thread_landed`, is the resumable alternative. The `wait_for_ready`
/// analogue.
pub(super) async fn wait_for_landed(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForLandedArgs = serde_json::from_value(args.clone())?;
    let Some(bus) = server.event_bus.as_ref() else {
        return Err(McpError::InvalidParams(
            "wait_for_landed requires an event bus".into(),
        ));
    };
    let wait = a
        .timeout_ms
        .unwrap_or(DEFAULT_WAIT_MS)
        .clamp(1, MAX_WAIT_MS);

    let filter = EventFilter {
        workspace_id: Some(auth.workspace_id),
        channel_id: a.channel_id.map(ChannelId),
        thread_id: a.thread_id.map(ThreadId),
        kinds: Some(std::collections::HashSet::from([EventKind::ThreadLanded])),
        ..EventFilter::default()
    };
    let mut stream = bus
        .subscribe(filter)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;

    let store = server.store.as_ref();

    // Lookback (Cluster 354): a land emitted in the gap before this subscribe is
    // still caught, RBAC-filtered like the live path.
    if let Some(since) = a.since_log_id {
        let kinds = std::collections::HashSet::from([EventKind::ThreadLanded]);
        if let Some(event) = lookback_event(
            store,
            auth,
            &kinds,
            a.channel_id.map(ChannelId),
            a.thread_id.map(ThreadId),
            since,
            true,
        )
        .await?
        {
            return Ok(content_json(&event));
        }
    }

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(wait as u64);
    loop {
        let item = match tokio::time::timeout_at(deadline, stream.next()).await {
            Err(_) | Ok(None) => return Ok(content_json(&Value::Null)),
            Ok(Some(item)) => item,
        };
        let maidan_bus::BusItem::Event(envelope) = item else {
            continue;
        };
        // Don't reveal a land in a thread the caller can't access.
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
