//! Required-reviewers MCP tools (Cluster 375.4, Wave 2 #22, G5/G-dev-5). An
//! orchestrator sets a thread's review requirement + names reviewers; a reviewer
//! agent submits an approve / request-changes decision; anyone can read the
//! review status. The FSM close-gate (Cluster 375.2) enforces it. The REST twin
//! is Cluster 375.3. Governance writes = `thread:transition`; reads =
//! `workspace:read`. Thread access is enforced by the pre-dispatch `thread_id`
//! gate.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{MemberId, ReviewDecision, ThreadId};
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct SetRequirementArgs {
    thread_id: uuid::Uuid,
    required_count: i64,
}

/// Set (upsert) a thread's review requirement — `required_count` distinct
/// qualifying approvals before it can `close` (Cluster 375.4).
pub(super) async fn set_review_requirement(
    store: &Arc<dyn Store>,
    _auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetRequirementArgs = serde_json::from_value(args.clone())?;
    if a.required_count < 0 {
        return Err(McpError::InvalidParams(
            "required_count must be >= 0".into(),
        ));
    }
    let req = store
        .set_review_requirement(ThreadId(a.thread_id), a.required_count)
        .await?;
    Ok(content_json(&req))
}

#[derive(Deserialize)]
struct AddReviewerArgs {
    thread_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

/// Name a reviewer for a thread — the eligible set (Cluster 375.4). Idempotent.
pub(super) async fn add_reviewer(
    store: &Arc<dyn Store>,
    _auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AddReviewerArgs = serde_json::from_value(args.clone())?;
    let added = store
        .add_reviewer(ThreadId(a.thread_id), MemberId(a.member_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "added": added })))
}

#[derive(Deserialize)]
struct SubmitReviewArgs {
    thread_id: uuid::Uuid,
    decision: ReviewDecision,
    #[serde(default)]
    note: Option<String>,
}

/// Submit a review decision as the caller (Cluster 375.4). An owner/assignee may
/// submit but it won't count toward the requirement (separation of duties).
pub(super) async fn submit_review(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SubmitReviewArgs = serde_json::from_value(args.clone())?;
    let note = a.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let review = store
        .submit_review(ThreadId(a.thread_id), auth.member_id, a.decision, note)
        .await?;
    Ok(content_json(&review))
}

#[derive(Deserialize)]
struct ThreadArg {
    thread_id: uuid::Uuid,
}

/// The thread's review status — `{required_count, approvals, approvals_met}`
/// (Cluster 375.4). The approval side of the close-gate; a `refutes` edge is
/// checked separately at the gate.
pub(super) async fn get_review_status(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let status = store.review_status(ThreadId(a.thread_id)).await?;
    Ok(content_json(&status))
}

/// List a thread's review decisions (Cluster 375.4).
pub(super) async fn list_reviews(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let reviews = store.list_reviews(ThreadId(a.thread_id)).await?;
    Ok(content_json(&reviews))
}
