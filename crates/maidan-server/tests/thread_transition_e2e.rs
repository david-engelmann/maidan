//! HTTP thread transitions publish `ThreadStateChanged`.

use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{Event, EventFilter, EventKind};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn transition_thread_publishes_thread_state_changed() {
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
    let mut subscriber = bus
        .subscribe(EventFilter::all().with_kinds([EventKind::ThreadStateChanged]))
        .await
        .unwrap();

    let app = router(AppState::for_tests(store, artifacts, bus.clone(), search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "tr-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let actor: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "actor", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let actor_id = actor["id"].as_str().unwrap();

    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "ch"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();

    let thread: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "work"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = thread["id"].as_str().unwrap();
    assert_eq!(thread["state"], "open");

    let updated: serde_json::Value = client
        .post(format!("{base}/threads/{thread_id}"))
        .json(&json!({"actor_id": actor_id, "action": "start_review"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["state"], "in_review");

    let event = tokio::time::timeout(Duration::from_secs(2), subscriber.next())
        .await
        .expect("timeout waiting for ThreadStateChanged")
        .expect("subscriber ended without event");
    let BusItem::Event(envelope) = event else {
        panic!("expected event, got lag or end");
    };
    match envelope.event {
        Event::ThreadStateChanged {
            from_state,
            to_state,
            ..
        } => {
            assert_eq!(from_state, maidan_types::ThreadState::Open);
            assert_eq!(to_state, maidan_types::ThreadState::InReview);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let conflict = client
        .post(format!("{base}/threads/{thread_id}"))
        .json(&json!({"actor_id": actor_id, "action": "archive"}))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);
}
