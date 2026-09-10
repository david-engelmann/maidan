//! Thread dispatch priority (Cluster 365, G3 fair dispatch): set/get over REST.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn thread_priority_set_and_get_over_rest() {
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

    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    // Auth ENABLED: the set handler persists `auth.member_id` (set_by FK), which
    // the nil-member `for_tests` bypass would FK-fail.
    let state = AppState::new(
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
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let agent = store
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
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: agent.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("w".into()),
            capabilities: vec!["workspace:read".into(), "thread:transition".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());
    let url = format!("{base}/threads/{}/priority", thread.id.0);

    // No explicit priority yet → 404 (absence = default 0).
    assert_eq!(
        client
            .get(&url)
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    // Set a priority.
    let put = client
        .put(&url)
        .header("Authorization", &auth)
        .json(&serde_json::json!({ "priority": 7 }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let p: serde_json::Value = put.json().await.unwrap();
    assert_eq!(p["priority"], 7);
    assert_eq!(p["thread_id"], thread.id.0.to_string());
    assert_eq!(p["set_by"], agent.id.0.to_string());

    // GET returns it; re-setting upserts (incl. a negative).
    let got: serde_json::Value = client
        .get(&url)
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["priority"], 7);
    let re: serde_json::Value = client
        .put(&url)
        .header("Authorization", &auth)
        .json(&serde_json::json!({ "priority": -2 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(re["priority"], -2);

    server.abort();
}
