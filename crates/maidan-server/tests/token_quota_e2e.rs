//! Per-token capability quotas return 429 when exceeded (Cluster 54).

use std::sync::{atomic::AtomicI64, Arc};

use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, TokenQuota};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

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
    let artifacts = Arc::new(maidan_artifacts::LocalFsStore::new(dir.path()));
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
    (addr, reqwest::Client::new(), store, server)
}

#[tokio::test]
async fn workspace_read_quota_returns_429_on_burst() {
    let (addr, client, store, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "quota-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    store
        .replace_token_quotas(
            token.id,
            &[TokenQuota {
                capability: capability::WORKSPACE_READ.into(),
                max_per_window: 2,
                window_secs: 60,
            }],
        )
        .await
        .unwrap();

    let auth = format!("Bearer {}", secret.as_str());
    for _ in 0..2 {
        let resp = client
            .get(format!("{base}/workspaces/{}", ws.id.0))
            .header("authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let limited = client
        .get(format!("{base}/workspaces/{}", ws.id.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);

    server.abort();
}

#[tokio::test]
async fn mint_api_token_accepts_quotas() {
    let (addr, client, store, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "mint-quota".into(),
        })
        .await
        .unwrap();
    let admin = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let admin_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: admin.id,
            app_installation_id: None,
            token_hash: hash_secret(admin_secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::TOKEN_ADMIN.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();

    let body: serde_json::Value = client
        .post(format!(
            "{base}/workspaces/{}/members/{}/tokens",
            ws.id.0, admin.id.0
        ))
        .header("authorization", format!("Bearer {}", admin_secret.as_str()))
        .json(&json!({
            "capabilities": ["workspace:read"],
            "quotas": [{
                "capability": "workspace:read",
                "max_per_window": 5,
                "window_secs": 30
            }]
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["quotas"][0]["max_per_window"], 5);

    server.abort();
}
