//! End-to-end: HTTP mutations publish events to the bus.

use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{EventBus, InMemoryBus};
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{Event, EventFilter, EventKind};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn http_mutations_publish_matching_events() {
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
    let bus = Arc::new(InMemoryBus::with_capacity(256));

    let mut subscriber = bus.subscribe(EventFilter::all()).await.unwrap();

    let app = router(AppState::new(
        store,
        artifacts,
        bus.clone(),
        search,
        true,
        true,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    // create workspace -> WorkspaceCreated
    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "evt-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap().to_string();

    // create member -> MemberJoined
    let alice: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap().to_string();

    // create channel -> ChannelCreated
    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap().to_string();

    // create thread -> ThreadCreated
    let th: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "kickoff"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap().to_string();

    // post message -> MessagePosted
    let _msg: serde_json::Value = client
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({"author_id": alice_id, "body": "hi"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // collect five events (one for each mutation)
    let mut events: Vec<Event> = Vec::new();
    let collect = async {
        while events.len() < 5 {
            if let Some(e) = subscriber.next().await {
                events.push(e);
            } else {
                break;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), collect)
        .await
        .expect("timed out collecting events");

    let kinds: Vec<EventKind> = events.iter().map(|e| e.kind()).collect();
    assert_eq!(
        kinds,
        vec![
            EventKind::WorkspaceCreated,
            EventKind::MemberJoined,
            EventKind::ChannelCreated,
            EventKind::ThreadCreated,
            EventKind::MessagePosted,
        ]
    );

    server.abort();
}
