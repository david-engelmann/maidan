//! The `/ui/api/me` capability-card source (Cluster 353.1): the `/ui` Session tab
//! reads the caller's *real* capabilities from `GET /me` (reused under the `/ui`
//! auth proxy) and renders `{can, can't}` = granted vs `known_capabilities −
//! granted`. A declared "allowed-tools" list is not a grant — this endpoint is
//! the ground truth. Driven with a bearer (the middleware accepts session OR
//! bearer); the DOM render is covered by `ui-tests/tests/session.spec.ts`.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn ui_api_me_reports_granted_and_withheld_capabilities() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace { name: "ui".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    // A deliberately narrow grant: read-only, so `workspace:write` lands in "can't".
    let token = mint(
        store.as_ref(),
        ws.id,
        member.id,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;

    let me: Value = client
        .get(format!("{base}/ui/api/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(me["member_id"], Value::from(member.id.0.to_string()));
    assert_eq!(me["workspace_id"], Value::from(ws.id.0.to_string()));
    assert_eq!(me["is_bearer"], Value::Bool(true));

    let granted: Vec<String> = serde_json::from_value(me["capabilities"].clone()).unwrap();
    assert_eq!(granted, vec![capability::WORKSPACE_READ.to_string()]);

    let known: Vec<String> = serde_json::from_value(me["known_capabilities"].clone()).unwrap();
    // The full vocabulary is present so the card can compute "can't" = known − granted.
    assert!(known.contains(&capability::WORKSPACE_READ.to_string()));
    assert!(known.contains(&capability::WORKSPACE_WRITE.to_string()));
    assert!(known.contains(&capability::CHANNEL_ADMIN.to_string()));
    // "can't" is non-empty for a narrow grant — the whole point of the card.
    let cant: Vec<&String> = known.iter().filter(|c| !granted.contains(c)).collect();
    assert!(cant
        .iter()
        .any(|c| c.as_str() == capability::WORKSPACE_WRITE));
    assert!(cant.len() >= known.len() - 1);
}
