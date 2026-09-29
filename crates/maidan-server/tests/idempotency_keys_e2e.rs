//! `Idempotency-Key` on writes: a retry with the same key and request gets
//! the first response back instead of running again; a different request
//! under the key is refused; a retry while the first runs gets 409; a
//! request that crashed while holding the key is taken over once its lock
//! lapses. Keys are per caller.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use axum::http::Method;
use chrono::{Duration, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{idempotency::fingerprint, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations, NewIdempotencyKey};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
use serde_json::Value;
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
    format!("Bearer {}", secret.as_str())
}

async fn spawn() -> (SocketAddr, Arc<dyn Store>) {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.keep()));
    let state = AppState::new(
        store.clone(),
        artifacts,
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED: keys belong to a real caller
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, store)
}

#[tokio::test]
async fn idempotency_keys_replay_refuse_and_take_over() {
    let (addr, store) = spawn().await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let ws = store
        .create_workspace(NewWorkspace {
            name: "idem".into(),
        })
        .await
        .unwrap();
    let member = |handle: &str| NewMember {
        workspace_id: ws.id,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Agent,
    };
    let alice = store.create_member(member("alice")).await.unwrap();
    let bob = store.create_member(member("bob")).await.unwrap();
    let alice_tok = mint(store.as_ref(), ws.id, alice.id).await;
    let bob_tok = mint(store.as_ref(), ws.id, bob.id).await;
    let path = format!("/workspaces/{}/channels", ws.id.0);
    let url = format!("{base}{path}");

    let post = |tok: &str, key: Option<&str>, body: &str| {
        let mut req = client
            .post(&url)
            .header("Authorization", tok)
            .header("Content-Type", "application/json")
            .body(body.to_string());
        if let Some(key) = key {
            req = req.header("Idempotency-Key", key);
        }
        req.send()
    };
    let channel_count = || async {
        let list: Vec<Value> = client
            .get(&url)
            .header("Authorization", &alice_tok)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        list.len()
    };

    // First request runs; the retry gets the same response, marked replayed,
    // and nothing new is created.
    let general = r#"{"name":"general"}"#;
    let first = post(&alice_tok, Some("k-1"), general).await.unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    assert!(first.headers().get("idempotent-replayed").is_none());
    let first: Value = first.json().await.unwrap();
    let retry = post(&alice_tok, Some("k-1"), general).await.unwrap();
    assert_eq!(retry.status(), StatusCode::CREATED);
    assert_eq!(retry.headers()["idempotent-replayed"], "true");
    assert!(retry.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    let retry: Value = retry.json().await.unwrap();
    assert_eq!(retry["id"], first["id"]);
    assert_eq!(channel_count().await, 1);

    // A different request under the same key is refused.
    let reused = post(&alice_tok, Some("k-1"), r#"{"name":"other"}"#)
        .await
        .unwrap();
    assert_eq!(reused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let problem: Value = reused.json().await.unwrap();
    assert!(problem["type"]
        .as_str()
        .unwrap()
        .ends_with("problems/idempotency-key-reused"));
    assert_eq!(channel_count().await, 1);

    // Malformed keys are refused before anything runs.
    for bad in ["has space", &"x".repeat(256)] {
        let res = post(&alice_tok, Some(bad), r#"{"name":"bad"}"#)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{bad:?}");
    }
    assert_eq!(channel_count().await, 1);

    // Keys are per caller: bob's `k-1` is his own.
    let bobs = post(&bob_tok, Some("k-1"), r#"{"name":"bobs"}"#)
        .await
        .unwrap();
    assert_eq!(bobs.status(), StatusCode::CREATED);
    assert!(bobs.headers().get("idempotent-replayed").is_none());
    assert_eq!(channel_count().await, 2);

    // Without a key a write runs every time.
    for name in ["a", "b"] {
        let res = post(&alice_tok, None, &format!(r#"{{"name":"{name}"}}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }
    assert_eq!(channel_count().await, 4);

    // A 4xx that says "no" is an answer, so it is kept and replayed; one that
    // says "not now" (here a 409 on the taken name) is released, so the
    // retry runs again.
    let nameless = r#"{"topic":"no name"}"#;
    let refused = post(&alice_tok, Some("k-bad"), nameless).await.unwrap();
    let status = refused.status();
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "{status}"
    );
    let refused_again = post(&alice_tok, Some("k-bad"), nameless).await.unwrap();
    assert_eq!(refused_again.status(), status);
    assert_eq!(refused_again.headers()["idempotent-replayed"], "true");
    let dup = post(&alice_tok, Some("k-dup"), general).await.unwrap();
    assert_eq!(dup.status(), StatusCode::CONFLICT);
    let dup_again = post(&alice_tok, Some("k-dup"), general).await.unwrap();
    assert_eq!(dup_again.status(), StatusCode::CONFLICT);
    assert!(dup_again.headers().get("idempotent-replayed").is_none());

    // A retry while the first request still holds the key gets 409; a
    // different request under a held key gets 422.
    let held = r#"{"name":"held"}"#;
    let now = Utc::now();
    store
        .reserve_idempotency_key(&NewIdempotencyKey {
            workspace_id: ws.id,
            actor_id: alice.id,
            key: "k-held".into(),
            fingerprint: fingerprint(&Method::POST, &path, held.as_bytes()),
            locked_until: now + Duration::minutes(5),
            expires_at: now + Duration::hours(24),
        })
        .await
        .unwrap();
    let busy = post(&alice_tok, Some("k-held"), held).await.unwrap();
    assert_eq!(busy.status(), StatusCode::CONFLICT);
    let busy_other = post(&alice_tok, Some("k-held"), r#"{"name":"x"}"#)
        .await
        .unwrap();
    assert_eq!(busy_other.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(channel_count().await, 4);

    // A request that crashed holding the key: once its lock lapses, the
    // retry runs.
    let crashed = r#"{"name":"crashed"}"#;
    store
        .reserve_idempotency_key(&NewIdempotencyKey {
            workspace_id: ws.id,
            actor_id: alice.id,
            key: "k-crashed".into(),
            fingerprint: fingerprint(&Method::POST, &path, crashed.as_bytes()),
            locked_until: now - Duration::seconds(1),
            expires_at: now + Duration::hours(24),
        })
        .await
        .unwrap();
    let taken = post(&alice_tok, Some("k-crashed"), crashed).await.unwrap();
    assert_eq!(taken.status(), StatusCode::CREATED);
    assert!(taken.headers().get("idempotent-replayed").is_none());
    assert_eq!(channel_count().await, 5);

    // SCIM answers in its own envelope, so the header is not interpreted
    // there: a malformed key does not turn into a problem+json 400.
    let scim = client
        .post(format!("{base}/scim/v2/Users"))
        .header("Authorization", &alice_tok)
        .header("Idempotency-Key", "has space")
        .header("Content-Type", "application/scim+json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_ne!(
        scim.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json")
    );

    // Reads ignore the header.
    let read = client
        .get(&url)
        .header("Authorization", &alice_tok)
        .header("Idempotency-Key", "has space")
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
}
