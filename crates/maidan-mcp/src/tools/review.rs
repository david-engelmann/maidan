//! Required-reviewers MCP tools. An orchestrator sets a thread's review
//! requirement + names reviewers; a reviewer agent submits an approve /
//! request-changes decision; anyone can read the review status. The FSM
//! close-gate enforces it. Mirrors the REST surface. Governance writes =
//! `thread:transition`; reads = `workspace:read`. Thread access is enforced by
//! the pre-dispatch `thread_id` gate.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{MemberId, ReviewDecision, ThreadId};
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetRequirementArgs {
    thread_id: uuid::Uuid,
    required_count: i64,
}

/// Set (upsert) a thread's review requirement — `required_count` distinct
/// qualifying approvals before it can `close`.
pub(super) async fn set_review_requirement(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetRequirementArgs = serde_json::from_value(args.clone())?;
    if a.required_count < 0 {
        return Err(McpError::InvalidParams(
            "required_count must be >= 0".into(),
        ));
    }
    let thread_id = ThreadId(a.thread_id);
    // A gate ratchets. Raising `k` is a tightening any transitioner may do;
    // lowering it — `0` included, which disarms the gate — is the waiver, and
    // answers to `channel:admin`. The dispatch capability is static per tool,
    // so the direction has to be checked here.
    // The store makes the decision inside the write, where a concurrent change
    // cannot turn a raise into a lowering; this only says who may lower.
    let allow_lower = auth.bypass
        || maidan_auth::require_observed_capability(
            auth,
            maidan_auth::AuthorizationSurface::Mcp,
            maidan_auth::capability::CHANNEL_ADMIN,
        )
        .is_ok();
    let actor = auth.actor_id;
    let (_, req) = store
        .set_review_requirement_audited(
            thread_id,
            a.required_count,
            allow_lower,
            Box::new(move |(from, req)| {
                let mut event = review_requirement_event(actor, *from, req);
                event.metadata["surface"] = json!("mcp");
                event
            }),
        )
        .await?;
    Ok(content_json(&req))
}

/// The record of a review-requirement write, as REST writes it: a lowering is
/// the waiver and keeps its own action.
fn review_requirement_event(
    actor: MemberId,
    from: i64,
    req: &maidan_types::ThreadReviewRequirement,
) -> maidan_types::NewAuditEvent {
    maidan_types::NewAuditEvent {
        actor_id: Some(actor),
        action: if req.required_count < from {
            "review_requirement.lower"
        } else {
            "review_requirement.set"
        }
        .into(),
        target_kind: Some("thread".into()),
        target_id: Some(req.thread_id.0),
        metadata: json!({ "from": from, "to": req.required_count }),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddReviewerArgs {
    thread_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

/// Name a reviewer for a thread — the eligible set. Idempotent. The thread is
/// access-checked at dispatch; the reviewer must be in its workspace, as on
/// REST.
pub(super) async fn add_reviewer(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AddReviewerArgs = serde_json::from_value(args.clone())?;
    if !auth.bypass {
        let thread = store.get_thread(ThreadId(a.thread_id)).await?;
        let channel = store.get_channel(thread.channel_id).await?;
        let reviewer = store.get_member(MemberId(a.member_id)).await?;
        if reviewer.workspace_id != channel.workspace_id {
            return Err(McpError::InvalidParams(
                "reviewer is not in this workspace".into(),
            ));
        }
    }
    let added = store
        .add_reviewer(ThreadId(a.thread_id), MemberId(a.member_id))
        .await?;
    Ok(content_json(&serde_json::json!({ "added": added })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitReviewArgs {
    thread_id: uuid::Uuid,
    decision: ReviewDecision,
    #[serde(default)]
    note: Option<String>,
}

/// Submit a review decision as the caller. An owner/assignee may submit but it
/// won't count toward the requirement (separation of duties). A change request
/// from the owner or a counting reviewer sends an `in_review` thread back to
/// `open` for rework.
pub(super) async fn submit_review(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SubmitReviewArgs = serde_json::from_value(args.clone())?;
    let note = a.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let thread_id = ThreadId(a.thread_id);
    let (review, reopened) = server
        .store
        .submit_review(thread_id, auth.member_id, a.decision, note)
        .await?;
    if let Some(stored) = reopened {
        server.publish_stored(&stored).await;
        let uris =
            crate::resource_updates::uris_for_thread_transition(server.store.as_ref(), thread_id)
                .await;
        server.publish_resource_uris(uris).await;
    }
    Ok(content_json(&review))
}

#[derive(Deserialize)]
struct ThreadArg {
    thread_id: uuid::Uuid,
}

/// The thread's review status — `{required_count, approvals, approvals_met}`.
/// The approval side of the close-gate; a `refutes` edge is checked separately
/// at the gate.
pub(super) async fn get_review_status(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let status = store.review_status(ThreadId(a.thread_id)).await?;
    Ok(content_json(&status))
}

/// List a thread's review decisions.
pub(super) async fn list_reviews(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let reviews = store.list_reviews(ThreadId(a.thread_id)).await?;
    Ok(content_json(&reviews))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use maidan_artifacts::LocalFsStore;
    use maidan_auth::{
        capability::{CHANNEL_ADMIN, THREAD_TRANSITION, TOKEN_ADMIN, WORKSPACE_READ},
        AuthContext,
    };
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
    use maidan_types::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    use crate::server::McpServer;

    /// These governance tools recorded nothing over MCP before 413.4; the REST
    /// twins did. And a reviewer from another workspace was accepted.
    #[tokio::test]
    async fn governance_tools_record_their_changes_and_stay_in_the_workspace() {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let workspace = |name: &'static str| {
            let store = store.clone();
            async move {
                let ws = store
                    .create_workspace(NewWorkspace { name: name.into() })
                    .await
                    .unwrap();
                let member = store
                    .create_member(NewMember {
                        workspace_id: ws.id,
                        handle: format!("{name}-m"),
                        display_name: None,
                        kind: MemberKind::Agent,
                    })
                    .await
                    .unwrap();
                (ws, member)
            }
        };
        let (ws, admin) = workspace("alpha").await;
        let (_, outsider) = workspace("bravo").await;
        let worker = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "worker".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "c".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: None,
            })
            .await
            .unwrap();
        store.require_land_gate(thread.id).await.unwrap();
        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::from_session(
            admin.id,
            ws.id,
            [
                THREAD_TRANSITION,
                CHANNEL_ADMIN,
                TOKEN_ADMIN,
                WORKSPACE_READ,
            ]
            .map(String::from)
            .to_vec(),
        );

        let t = thread.id.0;
        for (tool, args) in [
            (
                "set_review_requirement",
                json!({ "thread_id": t, "required_count": 2 }),
            ),
            (
                "set_review_requirement",
                json!({ "thread_id": t, "required_count": 1 }),
            ),
            ("clear_land_gate", json!({ "thread_id": t })),
            ("freeze_member", json!({ "member_id": worker.id.0 })),
            ("unfreeze_member", json!({ "member_id": worker.id.0 })),
        ] {
            server.call_tool(&auth, tool, &args).await.unwrap();
        }
        let actions: Vec<String> = store
            .list_audit(50)
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.metadata["surface"] == "mcp")
            .map(|row| row.action)
            .collect();
        for action in [
            "review_requirement.set",
            "review_requirement.lower",
            "land_gate.clear",
            "member.freeze",
            "member.unfreeze",
        ] {
            assert!(actions.iter().any(|a| a == action), "{action} unrecorded");
        }

        assert!(server
            .call_tool(
                &auth,
                "add_reviewer",
                &json!({ "thread_id": t, "member_id": outsider.id.0 }),
            )
            .await
            .is_err());
        assert!(store.list_reviewers(thread.id).await.unwrap().is_empty());
    }
}
