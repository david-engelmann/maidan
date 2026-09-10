//! The waiting-on-you inbox over REST (Cluster 368, Wave 2 #16): the route composes
//! a member's assigned non-terminal threads + the workspace's pending approval gates
//! (mentions are covered by the pure `assemble_waiting_inbox` unit test).
//! Auth-enabled + self-only, with a minted token that IS the acting member.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewApprovalGate, NewChannel, NewMember, NewThread, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn waiting_inbox_composes_assigned_threads_and_open_gates() {
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
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "worker".into(),
            display_name: None,
            kind: MemberKind::Human,
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
    // An assigned, non-terminal thread.
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("do the thing".into()),
        })
        .await
        .unwrap();
    store.assign_thread(thread.id, member.id).await.unwrap();
    // A pending approval gate.
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: member.id,
            prompt: "approve the deploy".into(),
            schema: None,
        })
        .await
        .unwrap();

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("w".into()),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());

    let inbox: serde_json::Value = client
        .get(format!("{base}/members/{}/waiting", member.id.0))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(inbox["total"], 2, "one assigned thread + one open gate");
    let kinds: Vec<&str> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"assigned_thread"));
    assert!(kinds.contains(&"open_gate"));
    assert_eq!(inbox["sla_secs"], 86400);
    // Freshly-created items are not yet overdue (the overdue math is unit-tested
    // against aged items in `assemble_waiting_inbox`).
    assert_eq!(inbox["overdue"], 0);

    server.abort();
}
