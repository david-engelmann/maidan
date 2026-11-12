//! End-to-end: HTTP mutations publish events to the bus.

use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_server::{router, test_support, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{Event, EventFilter, EventKind, MemberKind};
use serde_json::json;

#[tokio::test]
async fn http_mutations_publish_matching_events() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(256));

    // The workspace and member are seeded through the store: the HTTP routes
    // that create them are bootstrap's, closed here.
    let tenant = test_support::tenant(store.as_ref(), "evt-ws").await;
    let alice = tenant
        .member(store.as_ref(), "alice", MemberKind::Human)
        .await;

    let mut subscriber = bus.subscribe(EventFilter::all()).await.unwrap();

    let app = router(AppState::for_tests(store, artifacts, bus.clone(), search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = tenant.client();
    let base = format!("http://{addr}");
    let workspace_id = tenant.workspace_id.0;

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
    let _msg: serde_json::Value = alice
        .client()
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({"body": "hi"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // collect three events (one for each mutation)
    let mut events: Vec<Event> = Vec::new();
    let collect = async {
        while events.len() < 3 {
            if let Some(BusItem::Event(envelope)) = subscriber.next().await {
                events.push(envelope.event);
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
            EventKind::ChannelCreated,
            EventKind::ThreadCreated,
            EventKind::MessagePosted,
        ]
    );

    server.abort();
}
