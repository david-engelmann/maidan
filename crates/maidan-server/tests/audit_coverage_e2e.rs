//! Security-audit coverage (Cluster 182): token mint/revoke and channel
//! membership grants land in the audit trail, written through the real HTTP
//! handlers with auth enabled.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    SocketAddr,
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    // auth ENABLED — the fifth positional arg is `auth_disabled`.
    let mut state = AppState::new(
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
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store, server)
}

#[tokio::test]
async fn token_mint_revoke_and_membership_grants_are_audited() {
    let (addr, client, store, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "audit-cov".into(),
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
    let bob = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "private-room".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();

    // Bootstrap an operator token that can mint, manage members, and read audit.
    let admin_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: admin.id,
            app_installation_id: None,
            token_hash: hash_secret(admin_secret.as_str()),
            label: None,
            capabilities: vec![
                capability::TOKEN_ADMIN.into(),
                capability::CHANNEL_ADMIN.into(),
                capability::WORKSPACE_READ.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = |req: reqwest::RequestBuilder| {
        req.header("authorization", format!("Bearer {}", admin_secret.as_str()))
    };

    // Mint a token for bob → `token.mint`.
    let mint = auth(
        client
            .post(format!(
                "{base}/workspaces/{}/members/{}/tokens",
                ws.id.0, bob.id.0
            ))
            .json(&serde_json::json!({ "capabilities": [capability::MESSAGE_POST] })),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(mint.status(), StatusCode::CREATED);
    let minted: serde_json::Value = mint.json().await.unwrap();
    let minted_token_id = minted["id"].as_str().unwrap().to_string();

    // Revoke it → `token.revoke`.
    let revoke = auth(client.delete(format!("{base}/tokens/{minted_token_id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::OK);

    // Grant bob membership of the private channel → `channel_member.add`.
    let grant = auth(
        client
            .post(format!("{base}/channels/{}/members", channel.id.0))
            .json(&serde_json::json!({ "member_id": bob.id.0 })),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(grant.status(), StatusCode::CREATED);

    // The workspace audit list surfaces all three.
    let audit = auth(client.get(format!("{base}/workspaces/{}/audit?limit=50", ws.id.0)))
        .send()
        .await
        .unwrap();
    assert_eq!(audit.status(), StatusCode::OK);
    let rows: Vec<serde_json::Value> = audit.json().await.unwrap();
    let actions: Vec<&str> = rows.iter().filter_map(|r| r["action"].as_str()).collect();
    assert!(
        actions.contains(&"token.mint"),
        "missing token.mint in {actions:?}"
    );
    assert!(
        actions.contains(&"token.revoke"),
        "missing token.revoke in {actions:?}"
    );
    assert!(
        actions.contains(&"channel_member.add"),
        "missing channel_member.add in {actions:?}"
    );

    // The mint row records who/what.
    let mint_row = rows.iter().find(|r| r["action"] == "token.mint").unwrap();
    assert_eq!(mint_row["target_kind"], "api_token");
    assert_eq!(
        mint_row["metadata"]["subject_member_id"].as_str().unwrap(),
        bob.id.0.to_string()
    );

    server.abort();
}
