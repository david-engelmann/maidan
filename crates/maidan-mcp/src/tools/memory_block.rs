//! Attachable labeled memory-block MCP tools (Cluster 373.3, Wave 2 #21, H11).
//! An agent creates/reads/rewrites a Letta-shaped memory block and attaches it
//! to a thread — the way a parent watches a child's result block without a
//! nested runtime. Blocks are addressed by `label` within the caller's
//! workspace (their within-workspace key). The REST twin is Cluster 373.2.

use std::sync::Arc;

use chrono::Utc;
use futures::StreamExt;
use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

const DEFAULT_WAIT_MS: i64 = 30_000;
const MAX_WAIT_MS: i64 = 300_000;

#[derive(Deserialize)]
struct CreateArgs {
    label: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    char_limit: Option<i64>,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    value: Option<String>,
}

/// Create a memory block (Cluster 373.3). Concurrent-safe on `(workspace, label)`
/// — re-creating a label returns the existing block. `owner_id` is the caller.
pub(super) async fn create_memory_block(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CreateArgs = serde_json::from_value(args.clone())?;
    let label = a.label.trim().to_string();
    if !maidan_types::is_valid_block_label(&label) {
        return Err(McpError::InvalidParams(
            "label must be non-empty, trimmed, and at most 128 chars".into(),
        ));
    }
    let block = store
        .create_memory_block(NewMemoryBlock {
            workspace_id: auth.workspace_id,
            label,
            description: a.description,
            char_limit: a.char_limit,
            read_only: a.read_only,
            value: a.value.unwrap_or_default(),
            owner_id: auth.member_id,
        })
        .await?;
    Ok(content_json(&block))
}

#[derive(Deserialize)]
struct LabelArgs {
    label: String,
}

/// Get a block by label within the caller's workspace — `null` if none. This is
/// the "watch a child's result block" read (poll it; `GET /mcp/stream` is the
/// at-least-once path once 373.4 adds the reactive event).
pub(super) async fn get_memory_block(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: LabelArgs = serde_json::from_value(args.clone())?;
    let block = store
        .get_memory_block_by_label(auth.workspace_id, a.label.trim())
        .await?;
    Ok(content_json(&block))
}

/// List the caller's workspace's memory blocks (Cluster 373.3).
pub(super) async fn list_memory_blocks(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let blocks = store.list_memory_blocks(auth.workspace_id).await?;
    Ok(content_json(&blocks))
}

#[derive(Deserialize)]
struct SetArgs {
    label: String,
    value: String,
}

/// Full-rewrite a block's value by label (last-writer-wins). A missing block →
/// `InvalidParams`; a read-only block or over-limit value → `InvalidParams` (the
/// store's `InvalidInput`).
pub(super) async fn set_memory_block_value(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetArgs = serde_json::from_value(args.clone())?;
    let block = server
        .store
        .get_memory_block_by_label(auth.workspace_id, a.label.trim())
        .await?
        .ok_or_else(|| McpError::InvalidParams("no such memory block".into()))?;
    let updated = server
        .store
        .set_memory_block_value(block.id, &a.value)
        .await?;
    // A "go fetch" pointer so a parent watching the block wakes (Cluster 373.4).
    if server.event_bus.is_some() {
        server
            .publish_event(Event::MemoryBlockUpdated {
                occurred_at: Utc::now(),
                workspace_id: updated.workspace_id,
                block_id: updated.id,
                label: updated.label.clone(),
                updated_by: auth.member_id,
            })
            .await;
    }
    Ok(content_json(&updated))
}

#[derive(Deserialize)]
struct AttachArgs {
    thread_id: uuid::Uuid,
    label: String,
}

/// Attach a block (by label) to a thread — sharing it so a parent can watch it.
/// Thread access is enforced by the pre-dispatch gate on `thread_id`.
pub(super) async fn attach_memory_block(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AttachArgs = serde_json::from_value(args.clone())?;
    let block = store
        .get_memory_block_by_label(auth.workspace_id, a.label.trim())
        .await?
        .ok_or_else(|| McpError::InvalidParams("no such memory block".into()))?;
    let attached = store
        .attach_memory_block(ThreadId(a.thread_id), block.id)
        .await?;
    Ok(content_json(&json!({ "attached": attached })))
}

#[derive(Deserialize)]
struct ThreadArgs {
    thread_id: uuid::Uuid,
}

/// The blocks attached to a thread (Cluster 373.3). Thread access is enforced by
/// the pre-dispatch gate.
pub(super) async fn list_thread_memory_blocks(
    store: &Arc<dyn Store>,
    _auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArgs = serde_json::from_value(args.clone())?;
    let blocks = store
        .list_thread_memory_blocks(ThreadId(a.thread_id))
        .await?;
    Ok(content_json(&blocks))
}

/// Detach a block (by id) from a thread. `block_id` is used directly (a detach is
/// a precise op the agent has the id for from `list_thread_memory_blocks`).
#[derive(Deserialize)]
struct DetachArgs {
    thread_id: uuid::Uuid,
    block_id: uuid::Uuid,
}

pub(super) async fn detach_memory_block(
    store: &Arc<dyn Store>,
    _auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: DetachArgs = serde_json::from_value(args.clone())?;
    let detached = store
        .detach_memory_block(ThreadId(a.thread_id), MemoryBlockId(a.block_id))
        .await?;
    Ok(content_json(&json!({ "detached": detached })))
}

#[derive(Deserialize)]
struct WaitArgs {
    label: String,
    /// Long-poll window in milliseconds (default 30 000, clamped 1 000–300 000).
    #[serde(default)]
    timeout_ms: Option<i64>,
}

/// Block until a memory block (by label) is rewritten — a `MemoryBlockUpdated`
/// event (Cluster 373.4) in the caller's workspace — or the timeout lapses.
/// Returns the block (with its fresh value) or `null` on timeout. This is how a
/// parent watches a child's result block without a nested runtime. **Live**
/// primitive: it only sees updates produced *after* it subscribes, so read the
/// current value with `get_memory_block` first (the `GET /mcp/stream` SSE
/// transport, `kinds=memory_block_updated`, is the resumable alternative).
pub(super) async fn wait_for_memory_block(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitArgs = serde_json::from_value(args.clone())?;
    let Some(bus) = server.event_bus.as_ref() else {
        return Err(McpError::InvalidParams(
            "wait_for_memory_block requires an event bus".into(),
        ));
    };
    let wait = a
        .timeout_ms
        .unwrap_or(DEFAULT_WAIT_MS)
        .clamp(1, MAX_WAIT_MS);
    let label = a.label.trim().to_string();

    // Blocks aren't channel/thread-scoped, so the filter pins workspace + kind;
    // the specific block is matched by label as events arrive.
    let filter = EventFilter {
        workspace_id: Some(auth.workspace_id),
        kinds: Some(std::collections::HashSet::from([
            EventKind::MemoryBlockUpdated,
        ])),
        ..EventFilter::default()
    };
    let mut stream = bus
        .subscribe(filter)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(wait as u64);
    loop {
        let item = match tokio::time::timeout_at(deadline, stream.next()).await {
            Err(_) | Ok(None) => return Ok(content_json(&Value::Null)),
            Ok(Some(item)) => item,
        };
        let maidan_bus::BusItem::Event(env) = item else {
            continue; // a lag marker — keep waiting on the same deadline.
        };
        if let Event::MemoryBlockUpdated { label: l, .. } = &env.event {
            if l == &label {
                let block = server
                    .store
                    .get_memory_block_by_label(auth.workspace_id, &label)
                    .await?;
                return Ok(content_json(&block));
            }
        }
    }
}
