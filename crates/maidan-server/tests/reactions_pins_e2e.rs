//! Reactions and pins HTTP round-trip.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, test_support, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::MemberKind;
use serde_json::{json, Value};

async fn spawn() -> (SocketAddr, Arc<dyn Store>, tokio::task::JoinHandle<()>) {
    let pool = maidan_search::sqlite_pool_options()
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, store, server)
}

#[tokio::test]
async fn reaction_and_pin_http_flow() {
    let (addr, store, server) = spawn().await;
    let base = format!("http://{addr}");

    let tenant = test_support::tenant(store.as_ref(), "rx-pin").await;
    let client = tenant.client();
    let workspace_id = tenant.workspace_id.0;
    let alice = tenant
        .member(store.as_ref(), "alice", MemberKind::Human)
        .await
        .client();

    let ch: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general", "private": false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();

    let th: Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "topic"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let msg: Value = alice
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({
            "body": "pin me"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let message_id = msg["id"].as_str().unwrap();

    alice
        .post(format!("{base}/messages/{message_id}/reactions"))
        .json(&json!({"emoji": "🎉"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let reactions: Value = client
        .get(format!("{base}/messages/{message_id}/reactions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reactions.as_array().unwrap().len(), 1);

    alice
        .post(format!("{base}/threads/{thread_id}/pins"))
        .json(&json!({"message_id": message_id}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let pins: Value = client
        .get(format!("{base}/threads/{thread_id}/pins"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pins.as_array().unwrap().len(), 1);
    assert_eq!(pins[0]["message_id"].as_str().unwrap(), message_id);

    alice
        .delete(format!("{base}/messages/{message_id}/reactions"))
        .json(&json!({"emoji": "🎉"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let reactions2: Value = client
        .get(format!("{base}/messages/{message_id}/reactions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(reactions2.as_array().unwrap().is_empty());

    alice
        .delete(format!("{base}/threads/{thread_id}/pins"))
        .json(&json!({"message_id": message_id}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let pins2: Value = client
        .get(format!("{base}/threads/{thread_id}/pins"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(pins2.as_array().unwrap().is_empty());

    server.abort();
}
