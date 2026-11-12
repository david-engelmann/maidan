//! Assignment read-side over HTTP: claim-next returns the thread then null;
//! list-mine reflects the claim.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, test_support, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewThread};
use reqwest::StatusCode;

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
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, store, server)
}

#[tokio::test]
async fn claim_next_then_list_mine_then_empty() {
    let (addr, store, server) = spawn().await;
    let base = format!("http://{addr}");

    let tenant = test_support::tenant(store.as_ref(), "q").await;
    let agent = tenant
        .member(store.as_ref(), "agent", MemberKind::Agent)
        .await;
    let client = agent.client();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: tenant.workspace_id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();

    // claim-next returns the thread.
    let claim = client
        .post(format!(
            "{base}/channels/{}/threads/claim-next",
            channel.id.0
        ))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(claim.status(), StatusCode::OK);
    let body: serde_json::Value = claim.json().await.unwrap();
    assert_eq!(body["title"].as_str().unwrap(), "task");
    assert_eq!(
        body["assignee_id"].as_str().unwrap(),
        agent.member_id.0.to_string()
    );
    let pin = body["pin"].as_object().expect("pin");
    assert!(
        pin["uri"].as_str().unwrap().starts_with("maidan:event/"),
        "claim pins the assignment event: {:?}",
        pin["uri"]
    );
    assert!(pin["content_hash"].as_str().unwrap().starts_with("sha256:"));

    // list-mine now shows it.
    let mine = client
        .get(format!(
            "{base}/members/{}/assigned-threads",
            agent.member_id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(mine.status(), StatusCode::OK);
    let threads: Vec<serde_json::Value> = mine.json().await.unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0]["title"].as_str().unwrap(), "task");

    // A second claim finds no unassigned work → null.
    let empty = client
        .post(format!(
            "{base}/channels/{}/threads/claim-next",
            channel.id.0
        ))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::OK);
    assert!(empty.json::<serde_json::Value>().await.unwrap().is_null());

    server.abort();
}
