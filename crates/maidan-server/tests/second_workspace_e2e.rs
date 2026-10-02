//! An operator can open a second workspace without `MAIDAN_BOOTSTRAP`.
//!
//! The new admin can work in that workspace and cannot read the operator
//! workspace, list instance holds, or mint the cross-tenant capabilities.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
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
        .max_connections(4)
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
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    Harness {
        addr,
        server,
        client,
        store,
        _dir: dir,
    }
}

async fn mint(
    store: &dyn Store,
    workspace: maidan_types::WorkspaceId,
    member: maidan_types::MemberId,
    caps: Vec<String>,
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: workspace,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("test".into()),
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn operator_opens_a_second_workspace_without_bootstrap() {
    let h = spawn().await;
    let base = h.base();
    let first = h
        .store
        .create_workspace(NewWorkspace {
            name: "first".into(),
        })
        .await
        .unwrap();
    let admin = h
        .store
        .create_member(NewMember {
            workspace_id: first.id,
            handle: "root".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let operator = mint(h.store.as_ref(), first.id, admin.id, capability::all()).await;
    let narrow = mint(
        h.store.as_ref(),
        first.id,
        admin.id,
        vec![
            capability::WORKSPACE_WRITE.into(),
            capability::TOKEN_ADMIN.into(),
        ],
    )
    .await;

    let anon = h
        .client
        .post(format!("{base}/operator/workspaces"))
        .json(&json!({"name": "second", "admin_handle": "ada"}))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);

    let denied = h
        .client
        .post(format!("{base}/operator/workspaces"))
        .header("authorization", format!("Bearer {narrow}"))
        .json(&json!({"name": "second", "admin_handle": "ada"}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denied_body = denied.text().await.unwrap();
    assert!(denied_body.contains("operator:global"), "{denied_body}");

    let bootstrap = h
        .client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "nope"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bootstrap.status(), StatusCode::FORBIDDEN);

    let blank = h
        .client
        .post(format!("{base}/operator/workspaces"))
        .header("authorization", format!("Bearer {operator}"))
        .json(&json!({"name": "  ", "admin_handle": "ada"}))
        .send()
        .await
        .unwrap();
    assert_eq!(blank.status(), StatusCode::BAD_REQUEST);

    let created = h
        .client
        .post(format!("{base}/operator/workspaces"))
        .header("authorization", format!("Bearer {operator}"))
        .json(&json!({"name": " second ", "admin_handle": " ada "}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: serde_json::Value = created.json().await.unwrap();
    let wid = created["workspace"]["id"].as_str().unwrap().to_string();
    assert_ne!(wid, first.id.0.to_string());
    assert_eq!(created["workspace"]["name"], "second");
    assert_eq!(created["member"]["handle"], "ada");
    assert_eq!(created["member"]["kind"], "human");
    let caps = created["token"]["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|c| c == "token:admin"));
    assert!(caps.iter().any(|c| c == "workspace:write"));
    assert!(!caps.iter().any(|c| c == "operator:global"));
    assert!(!caps.iter().any(|c| c == "audit:read-global"));
    let secret = created["token"]["secret"].as_str().unwrap();
    let mid = created["member"]["id"].as_str().unwrap();

    let own = h
        .client
        .get(format!("{base}/workspaces/{wid}"))
        .header("authorization", format!("Bearer {secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(own.status(), StatusCode::OK);

    let foreign = h
        .client
        .get(format!("{base}/workspaces/{}", first.id.0))
        .header("authorization", format!("Bearer {secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);

    let holds = h
        .client
        .get(format!("{base}/operator/legal-holds"))
        .header("authorization", format!("Bearer {secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(holds.status(), StatusCode::FORBIDDEN);

    let escalated = h
        .client
        .post(format!("{base}/workspaces/{wid}/members/{mid}/tokens"))
        .header("authorization", format!("Bearer {secret}"))
        .json(&json!({"capabilities": ["operator:global"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(escalated.status(), StatusCode::BAD_REQUEST);
    let escalated_body = escalated.text().await.unwrap();
    assert!(
        escalated_body.contains("operator:global"),
        "{escalated_body}"
    );

    let worker = h
        .client
        .post(format!("{base}/workspaces/{wid}/members/{mid}/tokens"))
        .header("authorization", format!("Bearer {secret}"))
        .json(&json!({"capabilities": ["workspace:read", "message:post"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(worker.status(), StatusCode::CREATED);

    let audit = h
        .client
        .get(format!("{base}/workspaces/{wid}/audit"))
        .header("authorization", format!("Bearer {secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(audit.status(), StatusCode::OK);
    let audit: serde_json::Value = audit.json().await.unwrap();
    assert!(
        audit
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["action"] == "token.mint"),
        "{audit}"
    );

    let again = h
        .client
        .post(format!("{base}/operator/workspaces"))
        .header("authorization", format!("Bearer {operator}"))
        .json(&json!({"name": "third", "admin_handle": "bea"}))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::CREATED);
    assert_eq!(h.store.count_workspaces().await.unwrap(), 3);

    h.shutdown().await;
}
