//! Capability-registry MCP tools (Cluster 233, Arc E): declare / list a member's
//! skills and set / list a task's required skills. The MCP twin of the Cluster
//! 232 REST endpoints, over the shared store. Skill routing (231) reads both to
//! gate `claim_next`.

use std::sync::Arc;

use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct MemberSkillArgs {
    member_id: uuid::Uuid,
    skill: String,
}

/// Declare a skill for a member (Cluster 233). `workspace:write`.
pub(super) async fn add_member_skill(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberSkillArgs = serde_json::from_value(args.clone())?;
    if a.skill.trim().is_empty() {
        return Err(McpError::InvalidParams("skill must not be empty".into()));
    }
    store
        .add_member_skill(MemberId(a.member_id), a.skill.trim())
        .await?;
    Ok(content_json(&serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct MemberIdArgs {
    member_id: uuid::Uuid,
}

/// A member's declared skills (Cluster 233). `workspace:read`.
pub(super) async fn list_member_skills(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArgs = serde_json::from_value(args.clone())?;
    let skills = store.list_member_skills(MemberId(a.member_id)).await?;
    Ok(content_json(&skills))
}

#[derive(Deserialize)]
struct ThreadSkillArgs {
    thread_id: uuid::Uuid,
    skill: String,
}

/// Add a required skill to a task (Cluster 233). `thread:transition`; channel
/// access enforced pre-dispatch (the `thread_id` arg).
pub(super) async fn add_thread_required_skill(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadSkillArgs = serde_json::from_value(args.clone())?;
    if a.skill.trim().is_empty() {
        return Err(McpError::InvalidParams("skill must not be empty".into()));
    }
    store
        .add_thread_required_skill(ThreadId(a.thread_id), a.skill.trim())
        .await?;
    Ok(content_json(&serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ThreadIdArgs {
    thread_id: uuid::Uuid,
}

/// A task's required skills (Cluster 233). `workspace:read`; channel access
/// enforced pre-dispatch (the `thread_id` arg).
pub(super) async fn list_thread_required_skills(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadIdArgs = serde_json::from_value(args.clone())?;
    let skills = store
        .list_thread_required_skills(ThreadId(a.thread_id))
        .await?;
    Ok(content_json(&skills))
}
