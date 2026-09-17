//! A derived token inherits every limit the parent carried.
//!
//! `attenuate` deliberately permits an equal capability list — a no-op re-issue
//! is a legitimate way to get a fresh secret. That makes any bound the parent
//! carried and the child did not into a way of shedding that bound by asking.
//! Two were being dropped: the app installation, and the per-token quotas.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewApp, NewAppInstallation, NewMember, NewWorkspace, TokenQuota,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

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
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    std::mem::forget(dir);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

/// The headline: an installed app derives a token, the workspace revokes the
/// installation, and the derived token must stop working. Before 397.7 the
/// derived token named no installation, so the revoke had nothing to check and
/// the app kept its access after being uninstalled.
#[tokio::test]
async fn a_derived_token_dies_with_the_app_installation() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let app = store
        .create_app(NewApp {
            workspace_id: ws.id,
            slug: "an-app".into(),
            name: "An App".into(),
            description: None,
            created_by: bot.id,
        })
        .await
        .unwrap();
    let install = store
        .create_app_installation(NewAppInstallation {
            app_id: app.id,
            workspace_id: ws.id,
            bot_member_id: bot.id,
            granted_capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::MESSAGE_POST.into(),
            ],
        })
        .await
        .unwrap();

    // The app's own bearer, tied to the installation.
    let app_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: bot.id,
            app_installation_id: Some(install.id),
            token_hash: hash_secret(app_secret.as_str()),
            label: Some("app".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();

    let derived: Value = client
        .post(format!("{base}/tokens/attenuate"))
        .bearer_auth(app_secret.as_str())
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ], "label": "derived" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let derived_secret = derived["secret"].as_str().expect("minted").to_string();

    // It works while the installation stands.
    let before = client
        .get(format!("{base}/workspaces/{}", ws.id.0))
        .bearer_auth(&derived_secret)
        .send()
        .await
        .unwrap();
    assert_eq!(before.status(), StatusCode::OK, "derived token works");

    // Uninstall.
    store.revoke_app_installation(install.id).await.unwrap();

    let after = client
        .get(format!("{base}/workspaces/{}", ws.id.0))
        .bearer_auth(&derived_secret)
        .send()
        .await
        .unwrap();
    assert_eq!(
        after.status(),
        StatusCode::UNAUTHORIZED,
        "a derived token must not outlive the installation it came from"
    );
}

/// Quotas are keyed on the token id, so a child with none is a child with no
/// throttle — reachable by re-issuing the same capability list.
#[tokio::test]
async fn a_derived_token_inherits_the_parents_quotas() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
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
    let secret = TokenSecret::generate();
    let parent = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("throttled".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let quota = TokenQuota {
        capability: capability::MESSAGE_POST.into(),
        max_per_window: 10,
        window_secs: 60,
    };
    store
        .replace_token_quotas(parent.id, std::slice::from_ref(&quota))
        .await
        .unwrap();

    // A no-op re-issue: the same capabilities, a fresh secret.
    let derived: Value = client
        .post(format!("{base}/tokens/attenuate"))
        .bearer_auth(secret.as_str())
        .json(&json!({
            "capabilities": [capability::WORKSPACE_READ, capability::MESSAGE_POST]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let derived_id =
        maidan_types::ApiTokenId(uuid::Uuid::parse_str(derived["id"].as_str().unwrap()).unwrap());

    let inherited = store.list_token_quotas(derived_id).await.unwrap();
    assert_eq!(
        inherited,
        vec![quota],
        "a re-issue must not shed the parent's throttle"
    );
    // And it is reported back, so the holder can see what it got.
    assert_eq!(derived["quotas"][0]["capability"], capability::MESSAGE_POST);
}

/// Revoking a token kills everything derived from it, and the derived
/// credential actually stops working — not merely gets a column set.
///
/// The parent link used to live only in audit metadata, so revocation could not
/// traverse it and a child outlived the credential it was minted from. The same
/// shape was closed earlier for app installations and quotas; this is the third
/// dimension.
#[tokio::test]
async fn a_derived_token_dies_with_its_parent() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "revoke-e2e".into(),
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "rev-agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();

    let parent_secret = TokenSecret::generate();
    let parent = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: agent.id,
            app_installation_id: None,
            token_hash: hash_secret(parent_secret.as_str()),
            label: Some("parent".into()),
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();

    let attenuate = |bearer: String| {
        let (base, client) = (base.clone(), client.clone());
        async move {
            let v: Value = client
                .post(format!("{base}/tokens/attenuate"))
                .bearer_auth(bearer)
                .json(&json!({ "capabilities": [capability::WORKSPACE_READ] }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            v["secret"].as_str().expect("minted").to_string()
        }
    };
    let child_secret = attenuate(parent_secret.as_str().to_string()).await;
    // Derived from the *child*, so this only survives a one-level cascade.
    let grandchild_secret = attenuate(child_secret.clone()).await;

    let reads = |bearer: String| {
        let (base, client, wsid) = (base.clone(), client.clone(), ws.id.0);
        async move {
            client
                .get(format!("{base}/workspaces/{wsid}"))
                .bearer_auth(bearer)
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(reads(child_secret.clone()).await, StatusCode::OK);
    assert_eq!(reads(grandchild_secret.clone()).await, StatusCode::OK);

    store.revoke_api_token(parent.id).await.unwrap();

    assert_eq!(
        reads(child_secret).await,
        StatusCode::UNAUTHORIZED,
        "a derived token must not outlive the credential it was minted from"
    );
    assert_eq!(
        reads(grandchild_secret).await,
        StatusCode::UNAUTHORIZED,
        "the cascade must be transitive — a one-level kill is the same leak \
         one generation down"
    );
}
