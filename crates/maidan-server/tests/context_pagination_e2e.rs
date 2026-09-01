//! Context export pagination (Cluster 82.0).

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn thread_context_pages_messages_by_cursor() {
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "page-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
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
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("paged".into()),
        })
        .await
        .unwrap();

    for i in 0..3 {
        store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: format!("msg-{i}"),
                metadata: serde_json::json!({}),
                content: None,
            })
            .await
            .unwrap();
    }

    let page1: serde_json::Value = client
        .get(format!("{base}/threads/{}/context", thread.id.0))
        .query(&[("message_limit", "2")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page1["messages"].as_array().unwrap().len(), 2);
    assert_eq!(page1["messages"][0]["body"], "msg-0");
    let cursor = page1["next_message_cursor"]
        .as_str()
        .expect("next_message_cursor");

    let page2: serde_json::Value = client
        .get(format!("{base}/threads/{}/context", thread.id.0))
        .query(&[("message_limit", "2"), ("message_cursor", cursor)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page2["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page2["messages"][0]["body"], "msg-2");
    assert!(page2["next_message_cursor"].is_null());
}

#[tokio::test]
async fn mcp_get_thread_context_honors_message_cursor() {
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "mcp-page".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
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
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    for i in 0..2 {
        store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: format!("m{i}"),
                metadata: serde_json::json!({}),
                content: None,
            })
            .await
            .unwrap();
    }

    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    let resp = client
        .post(format!("{base}/mcp"))
        .json(&init)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "get_thread_context",
            "arguments": {
                "thread_id": thread.id.0,
                "message_limit": 1
            }
        }
    });
    let body: serde_json::Value = client
        .post(format!("{base}/mcp"))
        .json(&call)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let result = &body["result"];
    let text = result["content"][0]["text"].as_str().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["messages"].as_array().unwrap().len(), 1);
    assert!(parsed["next_message_cursor"].is_string());
}
