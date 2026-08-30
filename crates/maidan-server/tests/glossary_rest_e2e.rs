//! Shared-glossary REST (Cluster 322): set/get/list/delete a workspace's canonical
//! term -> definition. Auth ENABLED (real token) because `created_by` is a NOT-NULL
//! FK to members — the nil-member bypass would FK-fail on set (the 228/232 pattern).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
            ],
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
async fn glossary_crud_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "g".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "editor".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let bearer = format!("Bearer {tok}");
    let g = format!("{base}/workspaces/{}/glossary", ws.id.0);

    // Define a term.
    let put = client
        .put(format!("{g}/TTL"))
        .header("Authorization", &bearer)
        .json(&json!({ "definition": "time to live", "aliases": ["expiry"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let body: Value = put.json().await.unwrap();
    assert_eq!(body["term"], json!("TTL"));
    assert_eq!(body["definition"], json!("time to live"));
    assert_eq!(body["aliases"], json!(["expiry"]));
    assert_eq!(body["created_by"], json!(member.id.0.to_string()));

    // List has it.
    let list: Value = client
        .get(&g)
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["term"], json!("TTL"));

    // Get one.
    let one: Value = client
        .get(format!("{g}/TTL"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["definition"], json!("time to live"));

    // Unknown term -> 404.
    let miss = client
        .get(format!("{g}/nope"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(miss.status(), StatusCode::NOT_FOUND);

    // Redefine (upsert) — definition changes, list stays at 1.
    let put2 = client
        .put(format!("{g}/TTL"))
        .header("Authorization", &bearer)
        .json(&json!({ "definition": "how long a cache entry stays valid" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put2.status(), StatusCode::OK);
    let one2: Value = client
        .get(format!("{g}/TTL"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        one2["definition"],
        json!("how long a cache entry stays valid")
    );
    assert_eq!(one2["aliases"], json!([])); // aliases omitted -> reset to empty
    let list2: Value = client
        .get(&g)
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        list2.as_array().unwrap().len(),
        1,
        "upsert did not duplicate"
    );

    // Empty definition -> 400.
    let bad = client
        .put(format!("{g}/BAD"))
        .header("Authorization", &bearer)
        .json(&json!({ "definition": "  " }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    // Delete is conditional.
    let del = client
        .delete(format!("{g}/TTL"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let del2 = client
        .delete(format!("{g}/TTL"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del2.status(), StatusCode::NOT_FOUND);
}
