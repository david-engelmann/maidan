//! `GET /threads/{tid}/votes` reads every vote in a thread in one call: per
//! message, a count of each kind plus the rows, only messages that hold a
//! vote, and the same access rule as reading the thread.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
    WorkspaceId,
};
use reqwest::{Method, StatusCode};
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
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
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> (MemberId, String) {
    let id = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id;
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: id,
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
    (id, format!("Bearer {}", secret.as_str()))
}

async fn call(
    client: &reqwest::Client,
    method: Method,
    url: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = client.request(method, url).header("Authorization", token);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn one_read_returns_counts_and_voters_per_message() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "votes".into(),
        })
        .await
        .unwrap()
        .id;
    let other_ws = store
        .create_workspace(NewWorkspace {
            name: "elsewhere".into(),
        })
        .await
        .unwrap()
        .id;
    let (alice, alice_h) = member(store.as_ref(), ws, "alice").await;
    let (bob, bob_h) = member(store.as_ref(), ws, "bob").await;
    let (_, carol_h) = member(store.as_ref(), other_ws, "carol").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for body in ["first", "second", "third"] {
        let m = store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: alice,
                body: body.into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
        ids.push(m.id.0);
    }
    let url = format!("{base}/threads/{}/votes", thread.id.0);

    let (s, empty) = call(&client, Method::GET, &url, &alice_h, None).await;
    assert_eq!(s, StatusCode::OK, "{empty}");
    assert_eq!(empty["thread_id"], json!(thread.id.0.to_string()));
    assert_eq!(empty["messages"], json!([]));

    for (who, msg, kind) in [
        (&alice_h, ids[0], "approve"),
        (&bob_h, ids[0], "approve"),
        (&bob_h, ids[0], "ack"),
        (&alice_h, ids[2], "request_changes"),
    ] {
        let (s, b) = call(
            &client,
            Method::POST,
            &format!("{base}/messages/{msg}/votes"),
            who,
            Some(json!({ "kind": kind })),
        )
        .await;
        assert!(s.is_success(), "{s} {b}");
    }

    let (s, got) = call(&client, Method::GET, &url, &bob_h, None).await;
    assert_eq!(s, StatusCode::OK, "{got}");
    let msgs = got["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2, "the unvoted message is left out: {got}");
    let first = msgs
        .iter()
        .find(|m| m["message_id"] == json!(ids[0].to_string()))
        .unwrap();
    assert_eq!(
        (
            first["approve"].as_u64(),
            first["request_changes"].as_u64(),
            first["ack"].as_u64()
        ),
        (Some(2), Some(0), Some(1))
    );
    let mut voters: Vec<(String, String)> = first["votes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["member_id"].as_str().unwrap().to_string(),
                v["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    voters.sort();
    let mut want = vec![
        (alice.0.to_string(), "approve".to_string()),
        (bob.0.to_string(), "ack".to_string()),
        (bob.0.to_string(), "approve".to_string()),
    ];
    want.sort();
    assert_eq!(voters, want);
    let third = msgs
        .iter()
        .find(|m| m["message_id"] == json!(ids[2].to_string()))
        .unwrap();
    assert_eq!(third["request_changes"].as_u64(), Some(1));
    assert_eq!(third["approve"].as_u64(), Some(0));

    let (s, _) = call(&client, Method::GET, &url, &carol_h, None).await;
    assert!(
        s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN,
        "another workspace cannot read the votes: {s}"
    );
    let missing = format!("{base}/threads/{}/votes", uuid::Uuid::new_v4());
    let (s, _) = call(&client, Method::GET, &missing, &alice_h, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
