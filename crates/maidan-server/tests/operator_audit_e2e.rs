//! `GET /operator/audit` — global cross-workspace audit query (Cluster 132).
//!
//! The capability *denial* path (no `audit:read-global` → 403) is covered by the
//! table-driven `http_capability_matrix_e2e`. This asserts the *allow* path: a
//! token holding the capability gets every audit event back, across workspaces.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability::AUDIT_READ_GLOBAL, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewApiToken, NewAuditEvent, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    Arc<dyn Store>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    // auth ENABLED — so the capability gate is actually exercised.
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
    (addr, reqwest::Client::new(), store, server, dir)
}

#[tokio::test]
async fn operator_audit_returns_all_events_with_capability() {
    let (addr, client, store, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    // A workspace + member to own the token.
    let ws = store
        .create_workspace(NewWorkspace {
            name: "audit-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "operator".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();

    // Two audit events (global rows).
    for action in ["audit.one", "audit.two"] {
        store
            .append_audit(NewAuditEvent {
                actor_id: Some(member.id),
                action: action.into(),
                target_kind: None,
                target_id: None,
                metadata: json!({"workspace_id": ws.id.0}),
            })
            .await
            .unwrap();
    }

    // Mint a token holding the global audit capability.
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![AUDIT_READ_GLOBAL.into()],
            expires_at: None,
        })
        .await
        .unwrap();

    let resp = client
        .get(format!("{base}/operator/audit"))
        .bearer_auth(secret.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rows: Vec<Value> = resp.json().await.unwrap();
    let actions: Vec<&str> = rows.iter().filter_map(|r| r["action"].as_str()).collect();
    assert!(actions.contains(&"audit.one"), "got {actions:?}");
    assert!(actions.contains(&"audit.two"), "got {actions:?}");

    server.abort();
}
