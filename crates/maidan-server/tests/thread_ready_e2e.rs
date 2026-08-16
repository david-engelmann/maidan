//! Cluster 222: closing the last blocking dependency of a task publishes a
//! `ThreadReady` for that task over the event bus — the reactive counterpart to
//! the pull-only `dependencies_satisfied` readiness query.

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
async fn closing_last_dependency_publishes_thread_ready() {
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
        .subscribe(EventFilter::all().with_kinds([EventKind::ThreadReady]))
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

    let post_json = |path: String, body: serde_json::Value| {
        let client = client.clone();
        async move {
            client
                .post(path)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        }
    };

    let ws = post_json(format!("{base}/workspaces"), json!({"name": "ready-ws"})).await;
    let workspace_id = ws["id"].as_str().unwrap();
    let actor = post_json(
        format!("{base}/workspaces/{workspace_id}/members"),
        json!({"handle": "actor", "kind": "human"}),
    )
    .await;
    let actor_id = actor["id"].as_str().unwrap();
    let ch = post_json(
        format!("{base}/workspaces/{workspace_id}/channels"),
        json!({"name": "q"}),
    )
    .await;
    let channel_id = ch["id"].as_str().unwrap();

    let task = post_json(
        format!("{base}/channels/{channel_id}/threads"),
        json!({"title": "task"}),
    )
    .await;
    let task_id = task["id"].as_str().unwrap();
    let dep = post_json(
        format!("{base}/channels/{channel_id}/threads"),
        json!({"title": "dep"}),
    )
    .await;
    let dep_id = dep["id"].as_str().unwrap();

    // task depends on dep.
    let add = client
        .post(format!("{base}/threads/{task_id}/dependencies"))
        .json(&json!({"depends_on_thread_id": dep_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), 204, "dependency added");

    // Drive dep to a terminal state: open -> in_review -> closed.
    for action in ["start_review", "close"] {
        let resp = client
            .post(format!("{base}/threads/{dep_id}"))
            .json(&json!({"actor_id": actor_id, "action": action}))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "transition {action}");
    }

    // The close unblocked task -> exactly one ThreadReady for task.
    let event = tokio::time::timeout(Duration::from_secs(2), subscriber.next())
        .await
        .expect("timeout waiting for ThreadReady")
        .expect("subscriber ended without event");
    let BusItem::Event(envelope) = event else {
        panic!("expected event, got lag or end");
    };
    match envelope.event {
        Event::ThreadReady {
            thread_id, thread, ..
        } => {
            assert_eq!(thread_id.0.to_string(), task_id, "ready thread is the task");
            assert_eq!(thread.id, thread_id);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
