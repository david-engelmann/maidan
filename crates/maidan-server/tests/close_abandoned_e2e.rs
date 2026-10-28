//! Close a thread that will not land over the wire:
//! `POST /threads/{id}/close-abandoned` closes the caller's own thread,
//! refuses a member who neither produced nor holds it, refuses a
//! `duplicate_of` from another workspace, and never reaches into another
//! workspace's thread.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace, ThreadId, ThreadState,
    WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (String, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
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
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), store)
}

const CAPS: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::THREAD_TRANSITION,
];

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    token: String,
}

async fn token(store: &dyn Store, ws: WorkspaceId, handle: &str) -> String {
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: CAPS.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let token = token(store, ws.id, "agent").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        ws: ws.id,
        channel: channel.id,
        token,
    }
}

async fn create_thread(base: &str, room: &Room, title: &str) -> ThreadId {
    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", room.channel.0))
        .bearer_auth(&room.token)
        .json(&json!({ "title": title }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let body: Value = response.json().await.unwrap();
    ThreadId(body["id"].as_str().unwrap().parse().unwrap())
}

async fn close(base: &str, token: &str, thread: ThreadId, body: Value) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/threads/{}/close-abandoned", thread.0))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_producer_closes_its_abandoned_thread_and_no_one_reaches_across_workspaces() {
    let (base, store) = spawn().await;
    let a = room(store.as_ref(), "close-e2e-a").await;
    let b = room(store.as_ref(), "close-e2e-b").await;
    let stranger = token(store.as_ref(), a.ws, "stranger").await;

    let t_mine = create_thread(&base, &a, "mine").await;
    let t_orig = create_thread(&base, &a, "the original").await;
    let t_b = create_thread(&base, &b, "b's thread").await;

    // An unknown reason never reaches the handler.
    let (status, _) = close(&base, &a.token, t_mine, json!({ "reason": "bored" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A member who neither produced nor holds the thread is refused.
    let (status, _) = close(&base, &stranger, t_mine, json!({ "reason": "abandoned" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Workspace B cannot close A's thread.
    let (status, _) = close(&base, &b.token, t_mine, json!({ "reason": "abandoned" })).await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "{status}"
    );

    // A thread in another workspace is not a duplicate target, and the
    // refusal reads the same as for a made-up id.
    let (status, foreign) = close(
        &base,
        &a.token,
        t_mine,
        json!({ "reason": "duplicate", "duplicate_of": t_b.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, random) = close(
        &base,
        &a.token,
        t_mine,
        json!({ "reason": "duplicate", "duplicate_of": uuid::Uuid::now_v7() }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(foreign["detail"], random["detail"]);
    assert_eq!(
        store.get_thread(t_mine).await.unwrap().state,
        ThreadState::Open
    );

    // The producer closes it as a duplicate of its own original.
    let (status, body) = close(
        &base,
        &a.token,
        t_mine,
        json!({ "reason": "duplicate", "duplicate_of": t_orig.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "closed");

    // B's thread is untouched.
    assert_eq!(
        store.get_thread(t_b).await.unwrap().state,
        ThreadState::Open
    );
}
