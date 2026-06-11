//! App OAuth authorization-code flow over HTTP, backed by the persisted code
//! store (Cluster 104.0.2): authorize → exchange happy path, single-use,
//! redirect_uri binding, and PKCE S256.

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

// base64url(SHA256("maidan-pkce-verifier-104")), the S256 challenge for that verifier.
const PKCE_VERIFIER: &str = "maidan-pkce-verifier-104";
const PKCE_CHALLENGE: &str = "WqnVSH04oT9DO3uSLHq2Mx8VbR5IZnHMojDldwX1ErQ";

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
            name: "oauth-ws".into(),
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
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws.id, secret.as_str().to_string())
}

async fn create_app(client: &reqwest::Client, addr: SocketAddr, wid: &str, auth: &str) -> String {
    let resp = client
        .post(format!("http://{addr}/workspaces/{wid}/apps"))
        .header("Authorization", auth)
        .json(&json!({"slug": "oauth-bot", "name": "OAuth Bot"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let app: serde_json::Value = resp.json().await.unwrap();
    app["id"].as_str().unwrap().to_string()
}

/// Mint a fresh authorization code (admin-gated) for `redirect_uri`.
async fn authorize(
    client: &reqwest::Client,
    addr: SocketAddr,
    wid: &str,
    app_id: &str,
    auth: &str,
) -> String {
    let resp = client
        .post(format!(
            "http://{addr}/workspaces/{wid}/apps/{app_id}/oauth/authorize"
        ))
        .header("Authorization", auth)
        .json(&json!({"redirect_uri": "https://app.example/cb", "state": "xyz"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["state"], "xyz");
    v["authorization_code"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn oauth_authorize_then_exchange_is_single_use() {
    let Some(h) = spawn().await else {
        return;
    };
    let (wid, admin_secret) = seed_admin_token(h.store.as_ref()).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let auth = format!("Bearer {admin_secret}");
    let wid = wid.0.to_string();
    let app_id = create_app(&client, h.addr, &wid, &auth).await;

    // A redirect_uri mismatch is rejected — and, like the legacy in-memory flow,
    // still burns the code (consume is atomic delete-then-validate).
    let code = authorize(&client, h.addr, &wid, &app_id, &auth).await;
    let mismatch = client
        .post(format!("http://{}/oauth/app/token", h.addr))
        .json(&json!({"code": code, "redirect_uri": "https://evil.example/cb"}))
        .send()
        .await
        .unwrap();
    assert_eq!(mismatch.status(), 400);

    // A fresh code exchanged correctly yields an app-scoped token for the bot.
    let code = authorize(&client, h.addr, &wid, &app_id, &auth).await;
    let exch = client
        .post(format!("http://{}/oauth/app/token", h.addr))
        .json(&json!({"code": code, "redirect_uri": "https://app.example/cb"}))
        .send()
        .await
        .unwrap();
    assert_eq!(exch.status(), 201);
    let exch: serde_json::Value = exch.json().await.unwrap();
    let app_secret = exch["secret"].as_str().unwrap();
    let ctx = maidan_auth::resolve_bearer(h.store.as_ref(), app_secret)
        .await
        .expect("app bearer resolves");
    assert!(ctx.app_installation_id.is_some());

    // A second exchange of the same code is rejected (single-use, store-enforced).
    let replay = client
        .post(format!("http://{}/oauth/app/token", h.addr))
        .json(&json!({"code": code, "redirect_uri": "https://app.example/cb"}))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 401);

    h.server.abort();
}

#[tokio::test]
async fn oauth_pkce_requires_matching_verifier() {
    let Some(h) = spawn().await else {
        return;
    };
    let (wid, admin_secret) = seed_admin_token(h.store.as_ref()).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let auth = format!("Bearer {admin_secret}");
    let wid = wid.0.to_string();
    let app_id = create_app(&client, h.addr, &wid, &auth).await;

    let mint = |challenge: bool| {
        let client = client.clone();
        let auth = auth.clone();
        let addr = h.addr;
        let wid = wid.clone();
        let app_id = app_id.clone();
        async move {
            let mut body = json!({"redirect_uri": "https://app.example/cb", "state": "s"});
            if challenge {
                body["code_challenge"] = json!(PKCE_CHALLENGE);
            }
            let resp = client
                .post(format!(
                    "http://{addr}/workspaces/{wid}/apps/{app_id}/oauth/authorize"
                ))
                .header("Authorization", &auth)
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 201);
            let v: serde_json::Value = resp.json().await.unwrap();
            v["authorization_code"].as_str().unwrap().to_string()
        }
    };

    // Missing verifier on a PKCE-bound code → 400.
    let code = mint(true).await;
    let no_verifier = client
        .post(format!("http://{}/oauth/app/token", h.addr))
        .json(&json!({"code": code, "redirect_uri": "https://app.example/cb"}))
        .send()
        .await
        .unwrap();
    assert_eq!(no_verifier.status(), 400);

    // Wrong verifier → 401.
    let code = mint(true).await;
    let wrong = client
        .post(format!("http://{}/oauth/app/token", h.addr))
        .json(&json!({
            "code": code,
            "redirect_uri": "https://app.example/cb",
            "code_verifier": "not-the-verifier"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

    // Correct verifier → 201.
    let code = mint(true).await;
    let ok = client
        .post(format!("http://{}/oauth/app/token", h.addr))
        .json(&json!({
            "code": code,
            "redirect_uri": "https://app.example/cb",
            "code_verifier": PKCE_VERIFIER
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 201);

    h.server.abort();
}
