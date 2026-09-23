//! REST management of a member's delivery email — `PUT`/`GET`/`DELETE
//! /members/:id/email`. Auth ENABLED with a minted bearer.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
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
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn set_get_delete_member_email() {
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
    let bus = Arc::new(InMemoryBus::with_capacity(16));

    let ws = store
        .create_workspace(NewWorkspace { name: "e".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "me".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;

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
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let bearer = format!("Bearer {tok}");
    let mid = member.id.0;

    // Unset → 404.
    let miss = client
        .get(format!("{base}/members/{mid}/email"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(miss.status(), StatusCode::NOT_FOUND);

    // A garbage address is rejected.
    let bad = client
        .put(format!("{base}/members/{mid}/email"))
        .header("Authorization", &bearer)
        .json(&json!({ "email": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    // Set a valid address.
    let set = client
        .put(format!("{base}/members/{mid}/email"))
        .header("Authorization", &bearer)
        .json(&json!({ "email": "me@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);
    let body: Value = set.json().await.unwrap();
    assert_eq!(body["email"], "me@example.com");

    // GET returns it.
    let got: Value = client
        .get(format!("{base}/members/{mid}/email"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["email"], "me@example.com");

    // DELETE clears it, then GET is 404 again.
    let del = client
        .delete(format!("{base}/members/{mid}/email"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let after = client
        .get(format!("{base}/members/{mid}/email"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::NOT_FOUND);
}

/// D-5 on the HTTP surface. Before the personal-state guard, only a session
/// caller was bound to its own member — a bearer skipped the check entirely, so
/// an ordinary `workspace:read` token could point any member's digest mail
/// wherever it liked.
#[tokio::test]
async fn a_bearer_cannot_rewrite_another_members_delivery_address() {
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

    let ws = store
        .create_workspace(NewWorkspace { name: "d5".into() })
        .await
        .unwrap();
    let mk = |handle: &'static str| {
        let store = store.clone();
        let ws = ws.id;
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
        }
    };
    let caller = mk("caller").await;
    let victim = mk("victim").await;
    let tok = mint(store.as_ref(), ws.id, caller.id).await;

    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(16)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let bearer = format!("Bearer {tok}");

    // Its own address: allowed.
    let own = client
        .put(format!("{base}/members/{}/email", caller.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "email": "caller@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(own.status(), StatusCode::OK);

    // Somebody else's: refused.
    let other = client
        .put(format!("{base}/members/{}/email", victim.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "email": "attacker@evil.test" }))
        .send()
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::FORBIDDEN);

    // And nothing was written.
    assert!(
        store.get_member_email(victim.id).await.unwrap().is_none(),
        "a refused request must not have written"
    );
}
