//! `find_threads` over MCP: the twin of `GET /workspaces/{id}/threads`.
//! It matches on task attributes, defaults to the caller's own workspace,
//! refuses a foreign one, and errors on a garbage filter rather than
//! silently returning everything.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability::WORKSPACE_READ, AuthContext};
use maidan_mcp::server::McpServer;
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

fn body(value: &Value) -> Value {
    serde_json::from_str(value["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn ids_of(value: &Value) -> Vec<String> {
    body(value)
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn find_threads_matches_attributes_and_stays_in_the_callers_workspace() {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));

    let mut workspaces = Vec::new();
    let mut members = Vec::new();
    let mut pi_threads = Vec::new();
    for name in ["mcp-find-a", "mcp-find-b"] {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "agent".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let ch = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "work".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let (thread, _event) = store
            .create_thread_with_footprint(
                NewThread {
                    channel_id: ch.id,
                    parent_thread_id: None,
                    title: Some("pi issue".into()),
                    description: Some("brief".into()),
                    created_by: None,
                },
                None,
                Some(TaskAttributes {
                    kind: TaskKind::Change,
                    engine: Some(TaskEngine::Pi),
                    linear_issue: Some("ENG-100".into()),
                    ..TaskAttributes::default()
                }),
                None,
            )
            .await
            .unwrap();
        workspaces.push(ws.id);
        members.push(member.id);
        pi_threads.push(thread.id.0.to_string());
    }
    let (ws_a, member_a) = (workspaces[0], members[0]);
    let ws_b = workspaces[1];

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(member_a, ws_a, vec![WORKSPACE_READ.to_string()]);
    let call = |args: Value| {
        let server = &server;
        let auth = &auth;
        async move { server.call_tool(auth, "find_threads", &args).await }
    };

    // Omitted workspace: the caller's own, so only A's thread matches.
    let out = call(json!({ "attr": { "linear_issue": "ENG-100" } }))
        .await
        .unwrap();
    assert_eq!(ids_of(&out), vec![pi_threads[0].clone()]);
    // The bare shorthand agrees.
    let out = call(json!({ "engine": "pi" })).await.unwrap();
    assert_eq!(ids_of(&out), vec![pi_threads[0].clone()]);
    // Naming the caller's own workspace explicitly is the same search.
    let out = call(json!({
        "workspace_id": ws_a.0,
        "attr": { "linear_issue": "ENG-100" }
    }))
    .await
    .unwrap();
    assert_eq!(ids_of(&out), vec![pi_threads[0].clone()]);
    // A foreign workspace is refused, not searched.
    assert!(call(json!({ "workspace_id": ws_b.0 })).await.is_err());
    // Garbage filters error rather than widening.
    for args in [
        json!({ "attr": { "engine": "robot" } }),
        json!({ "attr": { "engnie": "pi" } }),
        json!({ "attr": { "linear_issue": "not-a-key" } }),
        json!({ "order": "sideways" }),
    ] {
        assert!(call(args).await.is_err(), "args should be refused");
    }
}
