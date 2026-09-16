//! Log snapshot + since-LSN catch-up + chain verify (Cluster 393).
//!
//! Twins of REST `GET /workspaces/:wid/snapshot`,
//! `GET /workspaces/:wid/events/catch-up`, and
//! `GET /workspaces/:wid/events/verify`. Header reads are `workspace:read`;
//! `include_graph=true` needs `token:admin` (the graph is an export dump).

use std::sync::Arc;

use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
use maidan_store::{build_log_snapshot, catch_up_since, Store, StoreError, CATCH_UP_LIMIT};
use maidan_types::{ChainBreakReason, LogSnapshot, WorkspaceId};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotArgs {
    #[serde(default)]
    workspace_id: Option<Uuid>,
    #[serde(default)]
    include_graph: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatchUpArgs {
    #[serde(default)]
    workspace_id: Option<Uuid>,
    #[serde(default)]
    after_lsn: i64,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyArgs {
    #[serde(default)]
    workspace_id: Option<Uuid>,
}

fn workspace(auth: &AuthContext, id: Option<Uuid>) -> Result<WorkspaceId, McpError> {
    let workspace_id = WorkspaceId(id.unwrap_or(auth.workspace_id.0));
    auth.ensure_workspace(workspace_id)?;
    Ok(workspace_id)
}

fn cursor_too_old(err: StoreError, workspace_id: WorkspaceId) -> McpError {
    match err {
        StoreError::CursorTooOld {
            after_id,
            oldest_id,
        } => McpError::InvalidParams(format!(
            "cursor too old: after_id {after_id} is behind oldest retained event {oldest_id}; must refetch {}",
            LogSnapshot::path(workspace_id)
        )),
        other => McpError::from(other),
    }
}

fn chain_broken(break_at: Option<i64>, reason: Option<ChainBreakReason>) -> McpError {
    let reason = reason.unwrap_or(ChainBreakReason::MalformedHash);
    match break_at {
        Some(id) => McpError::InvalidParams(format!(
            "event log chain broken at id={id}: {}",
            reason.as_str()
        )),
        None => McpError::InvalidParams(format!("event log chain broken: {}", reason.as_str())),
    }
}

pub(super) async fn get_log_snapshot(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SnapshotArgs = serde_json::from_value(args.clone())?;
    let workspace_id = workspace(auth, a.workspace_id)?;
    if a.include_graph && !auth.bypass && !auth.has_capability(TOKEN_ADMIN) {
        return Err(McpError::Forbidden(
            "include_graph requires token:admin".into(),
        ));
    }
    let snap = build_log_snapshot(store.as_ref(), workspace_id, a.include_graph).await?;
    Ok(content_json(&snap))
}

pub(super) async fn catch_up_events(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CatchUpArgs = serde_json::from_value(args.clone())?;
    let workspace_id = workspace(auth, a.workspace_id)?;
    let limit = a.limit.unwrap_or(100).clamp(1, CATCH_UP_LIMIT);
    let page = catch_up_since(store.as_ref(), workspace_id, a.after_lsn, limit)
        .await
        .map_err(|e| cursor_too_old(e, workspace_id))?;
    if !page.ok() {
        return Err(chain_broken(page.chain.break_at, page.chain.reason));
    }
    Ok(content_json(&page))
}

pub(super) async fn verify_event_chain(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: VerifyArgs = serde_json::from_value(args.clone())?;
    let workspace_id = workspace(auth, a.workspace_id)?;
    let report = store.as_ref().verify_event_chain(workspace_id).await?;
    if !report.ok {
        return Err(chain_broken(report.break_at, report.reason));
    }
    Ok(content_json(&report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use maidan_artifacts::LocalFsStore;
    use maidan_auth::capability::{TOKEN_ADMIN, WORKSPACE_READ};
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use maidan_types::{
        verify_snapshot, MemberKind, NewChannel, NewMember, NewWorkspace, CATCH_UP_TYPE,
        LOG_SNAPSHOT_TYPE,
    };
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    use crate::server::McpServer;

    fn content(v: &Value) -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    async fn blank_store() -> (Arc<dyn Store>, sqlx::SqlitePool) {
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
        (Arc::new(SqliteStore::new(pool.clone())), pool)
    }

    fn mcp(store: Arc<dyn Store>, pool: sqlx::SqlitePool) -> McpServer {
        McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        )
    }

    async fn seed(store: &dyn Store) -> (WorkspaceId, maidan_types::MemberId) {
        let (ws, _) = store
            .create_workspace_with_event(NewWorkspace {
                name: "snap".into(),
            })
            .await
            .unwrap();
        let (member, _) = store
            .create_member_with_event(NewMember {
                workspace_id: ws.id,
                handle: "reader".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let _ = store
            .create_channel_with_event(NewChannel {
                workspace_id: ws.id,
                name: "c".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        (ws.id, member.id)
    }

    #[tokio::test]
    async fn snapshot_catch_up_verify_and_graph_gate() {
        let (store, pool) = blank_store().await;
        let (ws, member) = seed(store.as_ref()).await;
        let server = mcp(store.clone(), pool);
        let reader = AuthContext::from_session(member, ws, vec![WORKSPACE_READ.to_string()]);
        let admin = AuthContext::from_session(
            member,
            ws,
            vec![WORKSPACE_READ.to_string(), TOKEN_ADMIN.to_string()],
        );

        let header = content(
            &server
                .call_tool(&reader, "get_log_snapshot", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(header["$type"], LOG_SNAPSHOT_TYPE);
        assert!(header.get("graph").is_none());
        assert!(header["graph_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        let as_of = header["as_of_lsn"].as_i64().unwrap();

        let denied = server
            .call_tool(
                &reader,
                "get_log_snapshot",
                &json!({ "include_graph": true }),
            )
            .await
            .unwrap_err();
        assert!(denied.to_string().contains("token:admin"), "{denied}");

        let full = content(
            &server
                .call_tool(
                    &admin,
                    "get_log_snapshot",
                    &json!({ "include_graph": true }),
                )
                .await
                .unwrap(),
        );
        let snap: LogSnapshot = serde_json::from_value(full).unwrap();
        assert!(snap.graph.is_some());
        assert!(verify_snapshot(&snap).ok);

        let page = content(
            &server
                .call_tool(&reader, "catch_up_events", &json!({ "after_lsn": 0 }))
                .await
                .unwrap(),
        );
        assert_eq!(page["$type"], CATCH_UP_TYPE);
        assert_eq!(page["chain"]["ok"], true);
        assert!(page["events"].as_array().unwrap().len() >= 2);

        let caught = content(
            &server
                .call_tool(&reader, "catch_up_events", &json!({ "after_lsn": as_of }))
                .await
                .unwrap(),
        );
        assert_eq!(caught["chain"]["ok"], true);
        assert!(caught["events"].as_array().unwrap().is_empty());

        let report = content(
            &server
                .call_tool(&reader, "verify_event_chain", &json!({}))
                .await
                .unwrap(),
        );
        assert_eq!(report["ok"], true);
        assert!(report["checked"].as_u64().unwrap() >= 2);
    }

    #[tokio::test]
    async fn catch_up_pruned_prefix_fails_closed_with_snapshot_path() {
        let (store, pool) = blank_store().await;
        let (ws, member) = seed(store.as_ref()).await;
        let events = store.list_events_after(ws, 0, 50).await.unwrap();
        assert!(events.len() >= 3);
        let first = events[0].id;
        let floor = events[1].id;
        let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
        store.prune_events(cutoff, floor, 10).await.unwrap();

        let server = mcp(store, pool);
        let reader = AuthContext::from_session(member, ws, vec![WORKSPACE_READ.to_string()]);
        let err = server
            .call_tool(&reader, "catch_up_events", &json!({ "after_lsn": first }))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must refetch"), "{msg}");
        assert!(msg.contains(&LogSnapshot::path(ws)), "{msg}");
    }
}
