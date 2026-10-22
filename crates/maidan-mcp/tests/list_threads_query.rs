//! `list_threads` over MCP takes the same order and filters as the REST
//! list: `order=newest` reverses the walk, `since` bounds by creation,
//! `result_kind` and `status` filter before the limit, and a garbage
//! value is an InvalidParams error rather than a silent default view.

use maidan_store::test_support::SetResultClaiming;
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
async fn list_threads_orders_and_filters_over_mcp() {
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
    let ws = store
        .create_workspace(NewWorkspace {
            name: "mcp-list".into(),
        })
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
    let mut ids = Vec::new();
    for title in ["one", "two", "three", "four"] {
        let t = store
            .create_thread(NewThread {
                channel_id: ch.id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap();
        ids.push(t.id);
    }
    let review = json!({ "result_kind": "example.review.result/1", "verdict": "ok" });
    store
        .set_thread_result_claiming(ids[1], member.id, &review)
        .await
        .unwrap();
    store
        .set_thread_result_claiming(ids[3], member.id, &review)
        .await
        .unwrap();
    store.claim_thread(ids[3], member.id).await.unwrap();
    store
        .declare_thread_status(ids[3], DeclaredStatus::Done, "note".into(), member.id)
        .await
        .unwrap();
    let ids: Vec<String> = ids.iter().map(|i| i.0.to_string()).collect();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(member.id, ws.id, vec![WORKSPACE_READ.to_string()]);
    let call = |args: Value| {
        let server = &server;
        let auth = &auth;
        async move { server.call_tool(auth, "list_threads", &args).await }
    };

    // Default: creation order.
    let out = call(json!({ "channel_id": ch.id.0 })).await.unwrap();
    assert_eq!(ids_of(&out), ids);
    // order=newest: reversed.
    let out = call(json!({ "channel_id": ch.id.0, "order": "newest" }))
        .await
        .unwrap();
    let mut reversed = ids.clone();
    reversed.reverse();
    assert_eq!(ids_of(&out), reversed);
    // result_kind and status filters.
    let out = call(json!({ "channel_id": ch.id.0, "result_kind": "example.review.result/1" }))
        .await
        .unwrap();
    assert_eq!(ids_of(&out), vec![ids[1].clone(), ids[3].clone()]);
    let out = call(json!({ "channel_id": ch.id.0, "status": "done" }))
        .await
        .unwrap();
    assert_eq!(ids_of(&out), vec![ids[3].clone()]);
    // since: a moment after the fourth thread's creation matches nothing;
    // the third thread's own instant keeps it and the fourth.
    let third = store
        .get_thread(ThreadId(uuid::Uuid::parse_str(&ids[2]).unwrap()))
        .await
        .unwrap();
    let out = call(json!({ "channel_id": ch.id.0, "since": third.created_at.to_rfc3339() }))
        .await
        .unwrap();
    assert_eq!(ids_of(&out), ids[2..].to_vec());
    // Garbage values error out.
    for args in [
        json!({ "channel_id": ch.id.0, "order": "sideways" }),
        json!({ "channel_id": ch.id.0, "status": "bogus" }),
        json!({ "channel_id": ch.id.0, "since": "not-a-date" }),
    ] {
        assert!(call(args).await.is_err(), "args should be refused");
    }
}
