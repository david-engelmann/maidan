//! Attachable labeled memory-block MCP tools (Cluster 373.3, Wave 2 #21, H11).
//! An agent creates/reads/rewrites a Letta-shaped memory block and attaches it
//! to a thread — the way a parent watches a child's result block without a
//! nested runtime. Blocks are addressed by `label` within the caller's
//! workspace (their within-workspace key). The REST twin is Cluster 373.2.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{MemoryBlockId, NewMemoryBlock, ThreadId};
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

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
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetArgs = serde_json::from_value(args.clone())?;
    let block = store
        .get_memory_block_by_label(auth.workspace_id, a.label.trim())
        .await?
        .ok_or_else(|| McpError::InvalidParams("no such memory block".into()))?;
    let updated = store.set_memory_block_value(block.id, &a.value).await?;
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
