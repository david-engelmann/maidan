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

pub(super) async fn claim_thread(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ClaimThreadArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let member_id = MemberId(a.member_id);
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
struct WaitForResultArgs {
    thread_id: uuid::Uuid,
    /// Long-poll window in milliseconds (default 30 000, clamped 1 000–300 000).
    #[serde(default)]
    timeout_ms: Option<i64>,
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
