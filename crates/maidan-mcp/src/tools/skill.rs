//! Capability-registry MCP tools: declare / list a member's skills and set /
//! list a task's required skills. The MCP twins of the REST endpoints, over the
//! shared store. Skill routing reads both to gate `claim_next`.

use std::sync::Arc;

use maidan_auth::{capability::CHANNEL_ADMIN, AuthContext};
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberSkillArgs {
    member_id: uuid::Uuid,
    skill: String,
}

/// Declare a skill for a member. `workspace:write`.
pub(super) async fn add_member_skill(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberSkillArgs = serde_json::from_value(args.clone())?;
    if a.skill.trim().is_empty() {
        return Err(McpError::InvalidParams("skill must not be empty".into()));
    }
    // The REST twin's split, and it has to live here rather than in the
    // pre-dispatch gate: whether `member_id` is the caller's own state or an
    // administrative target depends on the *skill* argument, not the tool. A
    // routing tag is the member's own declaration. A governance skill is what a
    // gate reads as authority, so granting one is an operator conferring it on
    // someone else — `channel:admin`, which `maidan.agent.worker` does not carry.
    if !auth.bypass {
        let target = MemberId(a.member_id);
        if is_governance_skill(&a.skill) {
            maidan_auth::require_observed_capability(
                auth,
                maidan_auth::AuthorizationSurface::Mcp,
                CHANNEL_ADMIN,
            )
            .map_err(McpError::from)?;
            // The gate that no longer covers this tool was also what kept the
            // grant inside the caller's workspace. Same refusal either way, so
            // this does not reveal whether the member exists.
            let member = store
                .get_member(target)
                .await
                .map_err(|_| McpError::Forbidden("member_id is not yours".to_string()))?;
            if member.workspace_id != auth.workspace_id {
                return Err(McpError::Forbidden("member_id is not yours".to_string()));
            }
        } else if target != auth.member_id {
            return Err(McpError::Forbidden("member_id is not yours".to_string()));
        }
    }
    if is_governance_skill(&a.skill) {
        store
            .grant_governance_skill_audited(
                MemberId(a.member_id),
                a.skill.trim(),
                maidan_types::NewAuditEvent {
                    actor_id: Some(auth.actor_id),
                    action: "member_skill.grant_governance".into(),
                    target_kind: Some("member".into()),
                    target_id: Some(a.member_id),
                    metadata: serde_json::json!({ "skill": a.skill.trim(), "surface": "mcp" }),
                },
            )
            .await?;
    } else {
        store
            .add_member_skill(MemberId(a.member_id), a.skill.trim())
            .await?;
    }
    Ok(content_json(&serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberIdArgs {
    member_id: uuid::Uuid,
}

/// A member's declared skills. `workspace:read`.
pub(super) async fn list_member_skills(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MemberIdArgs = serde_json::from_value(args.clone())?;
    let skills = store.list_member_skills(MemberId(a.member_id)).await?;
    Ok(content_json(&skills))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadSkillArgs {
    thread_id: uuid::Uuid,
    skill: String,
}

/// Add a required skill to a task. `thread:transition`; channel access enforced
/// pre-dispatch (the `thread_id` arg).
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
#[serde(deny_unknown_fields)]
struct ThreadIdArgs {
    thread_id: uuid::Uuid,
}

/// A task's required skills. `workspace:read`; channel access enforced
/// pre-dispatch (the `thread_id` arg).
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
