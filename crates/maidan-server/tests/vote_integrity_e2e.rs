//! A member holds one verdict on a message and can take a vote back, over REST
//! with auth on. `approve` and `request_changes` replace each other, `ack`
//! stands beside either, a retract removes only the caller's own vote, and
//! each replaced or retracted vote leaves a `vote_retracted` event.

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

/// `(member, kind)` pairs, sorted, for the message's votes.
async fn held(client: &reqwest::Client, url: &str, token: &str) -> Vec<(String, String)> {
    let (s, votes) = call(client, Method::GET, url, token, None).await;
    assert_eq!(s, StatusCode::OK, "{votes}");
    let mut pairs: Vec<_> = votes
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
    pairs.sort();
    pairs
}

#[tokio::test]
async fn a_member_holds_one_verdict_and_can_take_it_back() {
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
        })
        .await
        .unwrap();
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: alice,
            body: "ship it?".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();
    let url = format!("{base}/messages/{}/votes", msg.id.0);
    let vote = |token: &str, kind: &str| {
        let (client, url, token, kind) = (
            client.clone(),
            url.clone(),
            token.to_string(),
            kind.to_string(),
        );
        async move {
            let (s, body) = call(
                &client,
                Method::POST,
                &url,
                &token,
                Some(json!({ "kind": kind })),
            )
            .await;
            assert_eq!(s, StatusCode::NO_CONTENT, "{body}");
        }
    };
    let retract = |token: &str, kind: &str| {
        let (client, url, token, kind) = (
            client.clone(),
            url.clone(),
            token.to_string(),
            kind.to_string(),
        );
        async move {
            call(
                &client,
                Method::DELETE,
                &url,
                &token,
                Some(json!({ "kind": kind })),
            )
            .await
        }
    };
    let (a, b) = (alice.0.to_string(), bob.0.to_string());
    let pair = |m: &str, k: &str| (m.to_string(), k.to_string());

    // A second verdict replaces the first; an ack stands beside it.
    vote(&alice_h, "approve").await;
    vote(&alice_h, "request_changes").await;
    vote(&alice_h, "ack").await;
    vote(&bob_h, "approve").await;
    assert_eq!(held(&client, &url, &alice_h).await, {
        let mut want = vec![
            pair(&a, "ack"),
            pair(&a, "request_changes"),
            pair(&b, "approve"),
        ];
        want.sort();
        want
    });

    // Bob cannot take back Alice's vote: a retract is keyed on the caller.
    let (s, _) = retract(&bob_h, "request_changes").await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(held(&client, &url, &alice_h)
        .await
        .contains(&pair(&a, "request_changes")));

    // Nor can a member of another workspace reach the message at all.
    let (s, _) = retract(&carol_h, "approve").await;
    assert!(
        matches!(s, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "{s}"
    );
    assert!(held(&client, &url, &alice_h)
        .await
        .contains(&pair(&b, "approve")));

    // Alice takes hers back, once; a second retract changes nothing.
    for _ in 0..2 {
        let (s, body) = retract(&alice_h, "request_changes").await;
        assert_eq!(s, StatusCode::NO_CONTENT, "{body}");
    }
    let mut want = vec![pair(&a, "ack"), pair(&b, "approve")];
    want.sort();
    assert_eq!(held(&client, &url, &alice_h).await, want);

    // Outside the closed set is a 400 before any write.
    let (s, _) = retract(&alice_h, "👍").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The replaced approve and the retracted request_changes are both in the
    // log, each attributed to Alice.
    let (s, events) = call(
        &client,
        Method::GET,
        &format!("{base}/workspaces/{}/events?types=vote_retracted", ws.0),
        &alice_h,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{events}");
    let retracted: Vec<_> = events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["payload"]["member_id"].as_str().unwrap().to_string(),
                e["payload"]["vote_kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        retracted,
        [pair(&a, "approve"), pair(&a, "request_changes")],
        "{events}"
    );
}
