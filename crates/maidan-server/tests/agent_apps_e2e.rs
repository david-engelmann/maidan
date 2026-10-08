//! Installed apps: register, install, mint app token, post as bot.

mod common;

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_auth::{capability, hash_secret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::prelude::*;
use serde_json::json;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    store: Arc<dyn Store>,
    _container: testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
}

async fn spawn() -> Option<Harness> {
    let (container, pool) = common::postgres_pool().await?;
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let dir = tempfile::tempdir().expect("tempdir");
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
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

async fn register_app(
    client: &reqwest::Client,
    h: &Harness,
    wid: maidan_types::WorkspaceId,
    auth: &str,
) -> String {
    let resp = client
        .post(format!("http://{}/workspaces/{}/apps", h.addr, wid.0))
        .header("Authorization", auth)
        .json(&json!({ "slug": "review-bot", "name": "Review Bot" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let app: serde_json::Value = resp.json().await.unwrap();
    app["id"].as_str().unwrap().to_string()
}

async fn install(
    client: &reqwest::Client,
    h: &Harness,
    wid: maidan_types::WorkspaceId,
    app_id: &str,
    auth: &str,
    granted: &[&str],
) -> reqwest::Response {
    client
        .post(format!(
            "http://{}/workspaces/{}/apps/{}/install",
            h.addr, wid.0, app_id
        ))
        .header("Authorization", auth)
        .json(&json!({ "granted_capabilities": granted }))
        .send()
        .await
        .unwrap()
}

async fn revoke(
    client: &reqwest::Client,
    h: &Harness,
    wid: maidan_types::WorkspaceId,
    iid: &str,
    auth: &str,
) {
    let resp = client
        .delete(format!(
            "http://{}/workspaces/{}/app-installations/{}",
            h.addr, wid.0, iid
        ))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Revoke-and-reinstall is how an installation's grants change: the re-install
/// reuses the bot member (id, handle, history), applies the new grants, and is
/// audited. Workspace B's app with the same slug keeps its own member.
#[tokio::test]
async fn a_revoked_app_reinstalls_onto_its_bot_member_with_new_grants() {
    let Some(h) = spawn().await else {
        return;
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let (wid, admin_secret) = seed_admin_token(h.store.as_ref()).await;
    let auth = format!("Bearer {admin_secret}");
    let app_id = register_app(&client, &h, wid, &auth).await;

    let first = install(&client, &h, wid, &app_id, &auth, &["workspace:read"]).await;
    assert_eq!(first.status(), 201);
    let first: serde_json::Value = first.json().await.unwrap();
    let first_id = first["id"].as_str().unwrap().to_string();
    let bot_id = first["bot_member_id"].as_str().unwrap().to_string();

    let while_active = install(&client, &h, wid, &app_id, &auth, &["message:post"]).await;
    assert_eq!(while_active.status(), 409);

    revoke(&client, &h, wid, &first_id, &auth).await;
    let second = install(
        &client,
        &h,
        wid,
        &app_id,
        &auth,
        &["workspace:read", "message:post"],
    )
    .await;
    assert_eq!(second.status(), 201, "{}", second.text().await.unwrap());
    let second: serde_json::Value = second.json().await.unwrap();
    let second_id = second["id"].as_str().unwrap().to_string();
    assert_ne!(second_id, first_id);
    assert_eq!(second["bot_member_id"].as_str().unwrap(), bot_id);
    assert_eq!(
        second["granted_capabilities"],
        json!(["workspace:read", "message:post"])
    );

    let bot = h
        .store
        .get_member(maidan_types::MemberId(bot_id.parse().unwrap()))
        .await
        .unwrap();
    assert_eq!(bot.handle, "app:review-bot");

    // The new grant is mintable under the new installation.
    let mint = client
        .post(format!(
            "http://{}/workspaces/{}/app-installations/{}/tokens",
            h.addr, wid.0, second_id
        ))
        .header("Authorization", &auth)
        .json(&json!({ "capabilities": ["message:post"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(mint.status(), 201);
    let mint: serde_json::Value = mint.json().await.unwrap();
    assert_eq!(mint["bot_member_id"].as_str().unwrap(), bot_id);

    let installs: Vec<_> = h
        .store
        .list_audit(500)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.action == "app_installation.install")
        .collect();
    assert_eq!(installs.len(), 2, "one row per install: {installs:?}");
    let reinstall = installs
        .iter()
        .find(|row| row.target_id.map(|id| id.to_string()) == Some(second_id.clone()))
        .expect("the re-install is audited");
    assert_eq!(reinstall.metadata["bot_member_reused"], json!(true));
    assert_eq!(
        reinstall.metadata["bot_member_id"].as_str().unwrap(),
        bot_id
    );
    assert!(reinstall.actor_id.is_some());

    // Another workspace's app with the same slug gets, and keeps, its own bot.
    let (wid_b, admin_b) = seed_admin_token(h.store.as_ref()).await;
    let auth_b = format!("Bearer {admin_b}");
    let app_b = register_app(&client, &h, wid_b, &auth_b).await;
    let in_b = install(&client, &h, wid_b, &app_b, &auth_b, &["workspace:read"]).await;
    assert_eq!(in_b.status(), 201);
    let in_b: serde_json::Value = in_b.json().await.unwrap();
    let bot_b = in_b["bot_member_id"].as_str().unwrap().to_string();
    assert_ne!(bot_b, bot_id);
    revoke(&client, &h, wid_b, in_b["id"].as_str().unwrap(), &auth_b).await;
    let b_again = install(&client, &h, wid_b, &app_b, &auth_b, &["workspace:read"]).await;
    assert_eq!(b_again.status(), 201);
    let b_again: serde_json::Value = b_again.json().await.unwrap();
    assert_eq!(b_again["bot_member_id"].as_str().unwrap(), bot_b);

    h.server.abort();
}

/// `token:admin` is per-workspace. An app grant is minted later without its
/// installer, so the grant, and every mint from it, stays within what the
/// caller could mint directly: no workspace admin reaches across tenants
/// through an app.
#[tokio::test]
async fn a_workspace_admin_cannot_grant_or_mint_cross_tenant_capabilities_through_an_app() {
    let Some(h) = spawn().await else {
        return;
    };
    let (wid, admin_secret) = seed_admin_token(h.store.as_ref()).await;
    let client = reqwest::Client::new();
    let auth = format!("Bearer {admin_secret}");
    let app_id = register_app(&client, &h, wid, &auth).await;

    for global in [capability::OPERATOR_GLOBAL, capability::AUDIT_READ_GLOBAL] {
        let resp = install(
            &client,
            &h,
            wid,
            &app_id,
            &auth,
            &["workspace:read", global],
        )
        .await;
        assert_eq!(resp.status(), 400, "installing with {global}");
    }

    // An operator may have granted more; the admin minting from it may not.
    let installed = h
        .store
        .install_app_audited(
            wid,
            maidan_types::AppId(uuid::Uuid::parse_str(&app_id).unwrap()),
            vec!["workspace:read".into(), capability::OPERATOR_GLOBAL.into()],
            Box::new(|installed| maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(installed.installation.workspace_id),
                actor_id: None,
                action: "app_installation.install".into(),
                target_kind: Some("app_installation".into()),
                target_id: Some(installed.installation.id.0),
                metadata: json!({}),
            }),
        )
        .await
        .unwrap();
    let mint = |body: serde_json::Value| {
        client
            .post(format!(
                "http://{}/workspaces/{}/app-installations/{}/tokens",
                h.addr, wid.0, installed.installation.id.0
            ))
            .header("Authorization", &auth)
            .json(&body)
            .send()
    };
    assert_eq!(
        mint(json!({})).await.unwrap().status(),
        400,
        "the full grant"
    );
    assert_eq!(
        mint(json!({ "capabilities": [capability::OPERATOR_GLOBAL] }))
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        mint(json!({ "capabilities": ["workspace:read"] }))
            .await
            .unwrap()
            .status(),
        201,
        "the rest of the grant is still mintable"
    );

    h.server.abort();
}
