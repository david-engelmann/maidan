//! Result-delivery MCP tools (Cluster 379.5): list a thread's per-target
//! delivery status and replay one onto the egress outbox. The twins of
//! `GET /threads/:id/deliveries` and `POST …/deliveries/:did/replay`.
//!
//! Replay re-checks the workspace allowlist (status is not policy) and does
//! not bump `armed_revision`. The worker rebuilds the body from the current
//! thread result; the snapshot we enqueue is a fallback.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::{replay_result_delivery, ResultDeliveryReplay, Store};
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadIdArgs {
    thread_id: uuid::Uuid,
}

/// List per-target delivery status for a thread's result. Empty is a valid
/// outcome (delivered nowhere). Thread access is enforced pre-dispatch.
pub(super) async fn list_result_deliveries(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadIdArgs = serde_json::from_value(args.clone())?;
    let rows = store.list_result_deliveries(ThreadId(a.thread_id)).await?;
    Ok(content_json(&rows))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayArgs {
    thread_id: uuid::Uuid,
    delivery_id: uuid::Uuid,
}

/// Re-enqueue one delivery. Unblessed stays skipped; unroutable is
/// InvalidParams. Thread access is enforced pre-dispatch.
pub(super) async fn replay_result_delivery_tool(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ReplayArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let delivery_id = ResultDeliveryId(a.delivery_id);
    let thread = store.get_thread(thread_id).await?;
    let channel = store.get_channel(thread.channel_id).await?;
    let workspace_id = channel.workspace_id;
    if !auth.bypass && workspace_id != auth.workspace_id {
        return Err(McpError::InvalidParams(
            "thread is not in the caller's workspace".into(),
        ));
    }
    let row = store
        .get_result_delivery_by_id(thread_id, delivery_id)
        .await?
        .ok_or(McpError::NotFound)?;
    let body = match row.target() {
        Some(ref target) => snapshot_body(store, thread_id, target).await,
        None => String::new(),
    };
    let replayed =
        replay_result_delivery(store.as_ref(), workspace_id, thread_id, delivery_id, body)
            .await?
            .ok_or(McpError::NotFound)?;
    let row = match replayed {
        ResultDeliveryReplay::Unroutable(_) => {
            return Err(McpError::InvalidParams(
                "this target cannot be delivered — the surface is unknown or unusable".into(),
            ));
        }
        ResultDeliveryReplay::Skipped(row) | ResultDeliveryReplay::Enqueued(row) => row,
    };
    if let Err(err) = store
        .append_audit(NewAuditEvent {
            actor_id: (!auth.bypass).then_some(auth.member_id),
            action: "result_delivery.replay".into(),
            target_kind: Some("result_delivery".into()),
            target_id: Some(row.id.0),
            metadata: serde_json::json!({
                "thread_id": row.thread_id.0,
                "surface": row.surface,
                "selector": row.selector,
                "status": row.status,
            }),
        })
        .await
    {
        tracing::error!(
            target: "audit",
            %err,
            action = "result_delivery.replay",
            "audit.write_failed"
        );
    }
    Ok(content_json(&row))
}

/// Best-effort snapshot the worker will replace with a live rebuild. Enough
/// that a send still has bytes if the result row is gone by then.
async fn snapshot_body(
    store: &Arc<dyn Store>,
    thread_id: ThreadId,
    target: &EgressTarget,
) -> String {
    let Ok(Some(stored)) = store.get_thread_result(thread_id).await else {
        return String::new();
    };
    let Some(waiter) = parse_waiter_result(&stored.result) else {
        return String::new();
    };
    if waiter.is_reviewed() {
        match target {
            EgressTarget::Github { .. } => waiter.rendered.unwrap_or_default(),
            EgressTarget::Slack { .. } => waiter
                .summary
                .clone()
                .unwrap_or_else(|| waiter.result_kind.clone()),
        }
    } else {
        format!(
            "Maidan could not deliver this result: the producer reported status `{}`, not `reviewed`.",
            waiter.status
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use maidan_artifacts::LocalFsStore;
    use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};
    use maidan_auth::AuthContext;
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    use crate::error::McpError;
    use crate::server::McpServer;

    fn unwrap_content(v: Value) -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn result_delivery_tools_list_and_replay() {
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace {
                name: "mcp-deliv".into(),
            })
            .await
            .unwrap();
        let agent = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("review".into()),
            })
            .await
            .unwrap();
        let server = McpServer::new(
            store.clone(),
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        let auth = AuthContext::from_session(
            agent.id,
            ws.id,
            vec![WORKSPACE_READ.to_string(), WORKSPACE_WRITE.to_string()],
        );

        let empty = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "list_result_deliveries",
                    &json!({ "thread_id": thread.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(empty, json!([]));

        let rev = chrono::Utc::now();
        let target = EgressTarget::Github {
            repo: "acme/widgets".into(),
            issue_number: 7,
        };
        let armed = store
            .arm_result_delivery(thread.id, &target, rev)
            .await
            .unwrap()
            .unwrap();
        store
            .mark_result_delivery_skipped(armed.id, "target not in the workspace egress allowlist")
            .await
            .unwrap();

        let listed = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "list_result_deliveries",
                    &json!({ "thread_id": thread.id.0 }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(listed[0]["status"], json!("skipped"));

        let skipped = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "replay_result_delivery",
                    &json!({
                        "thread_id": thread.id.0,
                        "delivery_id": armed.id.0
                    }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(
            skipped["status"],
            json!("skipped"),
            "unblessed replay stays skipped"
        );

        store
            .allow_egress_target(NewEgressTarget {
                workspace_id: ws.id,
                surface: EgressSurface::Github,
                selector: "acme/widgets".into(),
            })
            .await
            .unwrap();
        let replayed = unwrap_content(
            server
                .call_tool(
                    &auth,
                    "replay_result_delivery",
                    &json!({
                        "thread_id": thread.id.0,
                        "delivery_id": armed.id.0
                    }),
                )
                .await
                .unwrap(),
        );
        assert_eq!(replayed["status"], json!("pending"));
        let outbox = store
            .claim_next_due_egress(chrono::Utc::now(), 120)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outbox.kind, EgressKind::Result);

        let unknown = store
            .arm_unroutable_result_delivery(thread.id, "discord", "guild", rev)
            .await
            .unwrap()
            .unwrap();
        store
            .mark_result_delivery_skipped(unknown.id, "unknown surface 'discord'")
            .await
            .unwrap();
        let err = server
            .call_tool(
                &auth,
                "replay_result_delivery",
                &json!({
                    "thread_id": thread.id.0,
                    "delivery_id": unknown.id.0
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }
}
