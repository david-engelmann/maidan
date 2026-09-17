//! REST `GET /workspaces/:wid/events/verify` reports chain integrity and fails
//! closed (409) on a break.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability::WORKSPACE_READ, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    pool: sqlx::SqlitePool,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

async fn spawn() -> Harness {
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
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
        client: reqwest::Client::new(),
        store,
        pool,
        _dir: dir,
    }
}

#[tokio::test]
async fn verify_ok_then_tamper_is_409() {
    let h = spawn().await;
    let (ws, _) = h
        .store
        .create_workspace_with_event(NewWorkspace {
            name: "chain".into(),
        })
        .await
        .unwrap();
    let (member, stored) = h
        .store
        .create_member_with_event(NewMember {
            workspace_id: ws.id,
            handle: "reader".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    h.store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let token = secret.as_str().to_string();

    let ok = h
        .client
        .get(format!("{}/workspaces/{}/events/verify", h.base(), ws.id.0))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let report: serde_json::Value = ok.json().await.unwrap();
    assert_eq!(report["ok"], true);
    assert!(report["checked"].as_u64().unwrap() >= 2);
    assert_eq!(report["from_genesis"], true);

    let mut payload = stored.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(stored.id)
        .execute(&h.pool)
        .await
        .unwrap();

    let broken = h
        .client
        .get(format!("{}/workspaces/{}/events/verify", h.base(), ws.id.0))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(broken.status(), StatusCode::CONFLICT);
    let problem: serde_json::Value = broken.json().await.unwrap();
    assert_eq!(
        problem["type"],
        "https://maidan.dev/problems/event-log-broken"
    );
    assert!(problem["detail"]
        .as_str()
        .unwrap()
        .contains("content_hash_mismatch"));

    h.shutdown().await;
}
