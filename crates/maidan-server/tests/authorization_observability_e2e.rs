//! REST and MCP denials share the content-free authorization metric lane.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Harness {
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
    maidan_server::metrics::init();
    let app = router(AppState::new(
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
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        addr,
        server,
        store,
        _dir: dir,
    }
}

async fn token_without_capabilities(store: &dyn Store) -> (String, maidan_types::WorkspaceId) {
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "authorization-observability".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "limited".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: workspace.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![],
            expires_at: None,
        })
        .await
        .unwrap();
    (secret.as_str().to_string(), workspace.id)
}

#[tokio::test]
async fn rest_and_mcp_denials_share_fixed_cardinality_metrics() {
    let harness = spawn().await;
    let (token, workspace_id) = token_without_capabilities(harness.store.as_ref()).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{}", harness.addr);
    let bearer = format!("Bearer {token}");

    let rest = client
        .get(format!("{base}/workspaces/{}", workspace_id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(rest.status(), reqwest::StatusCode::FORBIDDEN);

    let mcp: serde_json::Value = client
        .post(format!("{base}/mcp"))
        .header("Authorization", &bearer)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "list_channels", "arguments": {}}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        mcp.get("error").is_some(),
        "expected MCP authorization error"
    );

    let metrics = client
        .get(format!("{base}/metrics"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        metrics.contains(
            "maidan_authorization_decisions_total{surface=\"rest\",action=\"workspace:read\",outcome=\"denied\",resource=\"workspace\"} 1"
        ),
        "missing REST authorization metric in:\n{metrics}"
    );
    assert!(
        metrics.contains(
            "maidan_authorization_decisions_total{surface=\"mcp\",action=\"workspace:read\",outcome=\"denied\",resource=\"workspace\"} 1"
        ),
        "missing MCP authorization metric in:\n{metrics}"
    );
    assert!(!metrics.contains(workspace_id.0.to_string().as_str()));
    assert!(!metrics.contains(token.as_str()));

    harness.server.abort();
}
