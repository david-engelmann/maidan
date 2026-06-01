//! Installed apps: register, install, mint app token, post as bot.

mod common;

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_auth::{capability, hash_secret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{PostgresStore, Store};
use serde_json::json;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    store: Arc<dyn Store>,
    _container: testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
}

async fn spawn() -> Option<Harness> {
    let (container, pool) = common::postgres_pool().await?;
    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let dir = tempfile::tempdir().ok()?;
    let artifacts = Arc::new(maidan_artifacts::LocalFsStore::new(dir.path()));

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
    maidan_server::metrics::init();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
    let addr = listener.local_addr().ok()?;
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    Some(Harness {
        addr,
        server,
        store,
        _container: container,
    })
}

async fn seed_admin_token(store: &dyn Store) -> (maidan_types::WorkspaceId, String) {
    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "apps-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(maidan_types::NewMember {
            workspace_id: ws.id,
            handle: "admin".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = maidan_auth::TokenSecret::generate();
    store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::TOKEN_ADMIN.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws.id, secret.as_str().to_string())
}

#[tokio::test]
async fn app_token_posts_message_with_subset_of_granted_capabilities() {
    let Some(h) = spawn().await else {
        return;
    };
    let (wid, admin_secret) = seed_admin_token(h.store.as_ref()).await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let auth = format!("Bearer {admin_secret}");

    let app_resp = client
        .post(format!("http://{}/workspaces/{}/apps", h.addr, wid.0))
        .header("Authorization", &auth)
        .json(&json!({
            "slug": "ci-bot",
            "name": "CI Bot",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(app_resp.status(), 201);
    let app: serde_json::Value = app_resp.json().await.unwrap();
    let app_id = app["id"].as_str().unwrap();

    let install_resp = client
        .post(format!(
            "http://{}/workspaces/{}/apps/{}/install",
            h.addr, wid.0, app_id
        ))
        .header("Authorization", &auth)
        .json(&json!({
            "granted_capabilities": ["workspace:read", "message:post"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(install_resp.status(), 201);
    let install: serde_json::Value = install_resp.json().await.unwrap();
    let iid = install["id"].as_str().unwrap();
    let bot_id = install["bot_member_id"].as_str().unwrap();

    let mint_resp = client
        .post(format!(
            "http://{}/workspaces/{}/app-installations/{}/tokens",
            h.addr, wid.0, iid
        ))
        .header("Authorization", &auth)
        .json(&json!({
            "capabilities": ["message:post"],
            "label": "ci-run"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(mint_resp.status(), 201);
    let mint: serde_json::Value = mint_resp.json().await.unwrap();
    let app_secret = mint["secret"].as_str().unwrap();

    let ctx = maidan_auth::resolve_bearer(h.store.as_ref(), app_secret)
        .await
        .expect("app bearer resolves");
    assert_eq!(
        ctx.app_installation_id.map(|i| i.0.to_string()),
        Some(iid.to_string())
    );
    assert_eq!(ctx.member_id.0.to_string(), bot_id);

    let bad_mint = client
        .post(format!(
            "http://{}/workspaces/{}/app-installations/{}/tokens",
            h.addr, wid.0, iid
        ))
        .header("Authorization", &auth)
        .json(&json!({
            "capabilities": ["workspace:write"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_mint.status(), 400);

    h.server.abort();
}
