//! Legal hold over REST (Cluster 366, T6): place/get/lift + purge is refused
//! (409) while held. Auth-enabled with a minted `token:admin` bearer (place
//! persists `placed_by` and records audit).

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
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn legal_hold_blocks_purge_over_rest() {
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

    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
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
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
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
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: admin.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("admin".into()),
            capabilities: vec![
                "workspace:read".into(),
                "workspace:write".into(),
                "token:admin".into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());
    let hold_url = format!("{base}/workspaces/{}/legal-hold", ws.id.0);
    let purge_url = format!("{base}/workspaces/{}/purge", ws.id.0);

    // No hold → 404, and purge is allowed (200).
    assert_eq!(
        client
            .get(&hold_url)
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    // Place a hold.
    let put = client
        .put(&hold_url)
        .header("Authorization", &auth)
        .json(&serde_json::json!({ "reason": "litigation hold #42" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let h: serde_json::Value = put.json().await.unwrap();
    assert_eq!(h["reason"], "litigation hold #42");
    assert_eq!(h["workspace_id"], ws.id.0.to_string());
    assert_eq!(h["placed_by"], admin.id.0.to_string());

    // GET returns it.
    assert_eq!(
        client
            .get(&hold_url)
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    // Operator list contains it.
    let list: serde_json::Value = client
        .get(format!("{base}/operator/legal-holds"))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Purge is refused with 409 while held.
    assert_eq!(
        client
            .post(&purge_url)
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );

    // Lift → 204, then 404, and purge is allowed again.
    assert_eq!(
        client
            .delete(&hold_url)
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client
            .delete(&hold_url)
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .post(&purge_url)
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    server.abort();
}
