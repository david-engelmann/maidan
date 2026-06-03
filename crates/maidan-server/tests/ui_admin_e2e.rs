//! UI v4 admin console: audit/peers session API, destructive ops markers.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewAuditEvent, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

fn federation_test_key() -> Option<Arc<[u8; 32]>> {
    Some(Arc::new([0x42; 32]))
}

async fn spawn() -> (
    std::net::SocketAddr,
    reqwest::Client,
    Arc<dyn Store>,
    tokio::task::JoinHandle<()>,
) {
    let pool = SqlitePoolOptions::new()
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
    let app = router(AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, federation_test_key()),
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
        None,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    (addr, client, store, server)
}

#[tokio::test]
async fn ui_v4_admin_shell_and_session_audit_api() {
    let (addr, client, store, server) = spawn().await;
    let base = format!("http://{addr}");

    let html = client
        .get(format!("{base}/ui/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains(r#"data-ui-version="6""#));
    assert!(html.contains(r#"data-tab="admin""#));
    assert!(html.contains("load-audit"));
    assert!(html.contains("purge-workspace"));
    assert!(html.contains("revoke-token"));

    let ws = store
        .create_workspace(NewWorkspace {
            name: "admin-ui".into(),
        })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store
        .append_audit(NewAuditEvent {
            actor_id: Some(alice.id),
            action: "operator.test".into(),
            target_kind: None,
            target_id: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let audit: Vec<serde_json::Value> = client
        .get(format!(
            "{base}/ui/api/workspaces/{}/audit?limit=10",
            ws.id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["action"], "operator.test");

    let peers: Vec<serde_json::Value> = client
        .get(format!("{base}/ui/api/workspaces/{}/peers", ws.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(peers.is_empty());

    let created = client
        .post(format!("{base}/workspaces/{}/peers", ws.id.0))
        .json(&json!({"name": "upstream", "base_url": "https://peer.example"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let peer_body: serde_json::Value = created.json().await.unwrap();
    assert!(peer_body["secret"].as_str().is_some());

    let peers: Vec<serde_json::Value> = client
        .get(format!("{base}/ui/api/workspaces/{}/peers", ws.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0]["name"], "upstream");

    server.abort();
}
