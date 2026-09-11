//! Named-secret management over HTTP (Cluster 371.2, Wave 2 #19). Runs with auth
//! ENABLED (the `created_by` FK + real cap checks) and an encryption key
//! configured, so the encrypt-on-create / decrypt-on-resolve round-trip is
//! exercised. The value crosses the wire only on create + resolve — a list
//! returns metadata with no value.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
use serde_json::{json, Value};
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
        FederationRuntime::new(true, Some(Arc::new([7u8; 32]))), // an at-rest key
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
async fn secret_create_list_resolve_rotate_delete_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let admin = mint(
        store.as_ref(),
        ws.id,
        member.id,
        vec![
            capability::SECRET_ADMIN.into(),
            capability::SECRET_READ.into(),
        ],
    )
    .await;
    let admin_h = format!("Bearer {admin}");

    // Create a secret.
    let created = client
        .post(format!("{base}/workspaces/{}/secrets", ws.id.0))
        .header("Authorization", &admin_h)
        .json(&json!({ "name": "api-key", "value": "s3cr3t" }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let meta: Value = created.json().await.unwrap();
    assert_eq!(meta["name"], "api-key");
    assert!(
        meta.get("value").is_none(),
        "create returns metadata, no value"
    );

    // List returns metadata only — no value field anywhere.
    let list: Value = client
        .get(format!("{base}/workspaces/{}/secrets", ws.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(list[0].get("value").is_none(), "list carries no value");

    // Resolve returns the decrypted value (the encrypt→decrypt round-trip).
    let resolved: Value = client
        .post(format!(
            "{base}/workspaces/{}/secrets/api-key/resolve",
            ws.id.0
        ))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolved["value"], "s3cr3t");

    // Rotate: re-create the same name with a new value; resolve reflects it.
    let rot = client
        .post(format!("{base}/workspaces/{}/secrets", ws.id.0))
        .header("Authorization", &admin_h)
        .json(&json!({ "name": "api-key", "value": "rotated" }))
        .send()
        .await
        .unwrap();
    assert_eq!(rot.status(), StatusCode::CREATED);
    let resolved2: Value = client
        .post(format!(
            "{base}/workspaces/{}/secrets/api-key/resolve",
            ws.id.0
        ))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolved2["value"], "rotated");
    assert_eq!(
        store.list_secrets(ws.id).await.unwrap().len(),
        1,
        "rotation is in place, not a duplicate"
    );

    // secret:read alone can resolve + list but NOT create (secret:admin).
    let reader = mint(
        store.as_ref(),
        ws.id,
        member.id,
        vec![capability::SECRET_READ.into()],
    )
    .await;
    let reader_h = format!("Bearer {reader}");
    let read_resolve = client
        .post(format!(
            "{base}/workspaces/{}/secrets/api-key/resolve",
            ws.id.0
        ))
        .header("Authorization", &reader_h)
        .send()
        .await
        .unwrap();
    assert_eq!(read_resolve.status(), StatusCode::OK);
    let read_create = client
        .post(format!("{base}/workspaces/{}/secrets", ws.id.0))
        .header("Authorization", &reader_h)
        .json(&json!({ "name": "nope", "value": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        read_create.status(),
        StatusCode::FORBIDDEN,
        "read can't create"
    );

    // Resolve a missing name → 404. Invalid name → 400.
    let missing = client
        .post(format!(
            "{base}/workspaces/{}/secrets/ghost/resolve",
            ws.id.0
        ))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let bad_name = client
        .post(format!("{base}/workspaces/{}/secrets", ws.id.0))
        .header("Authorization", &admin_h)
        .json(&json!({ "name": "has space", "value": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_name.status(), StatusCode::BAD_REQUEST);

    // Delete → gone (404 on repeat resolve).
    let del = client
        .delete(format!("{base}/workspaces/{}/secrets/api-key", ws.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let gone = client
        .post(format!(
            "{base}/workspaces/{}/secrets/api-key/resolve",
            ws.id.0
        ))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}
