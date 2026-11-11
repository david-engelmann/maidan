//! Close a thread that will not land over the wire:
//! `POST /threads/{id}/close-abandoned` closes the caller's own thread,
//! refuses a member who neither produced nor holds it, refuses a
//! `duplicate_of` from another workspace, and never reaches into another
//! workspace's thread. A thread created with `supersedes` closes the one it
//! replaces the same way, under the same authority.

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

async fn spawn() -> (String, Arc<dyn Store>) {
    let pool = maidan_search::sqlite_pool_options()
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

async fn create(base: &str, token: &str, room: &Room, body: Value) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", room.channel.0))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

fn ticket() -> Value {
    json!({ "kind": "change", "linear_issue": "ENG-7", "repo": "acme/app" })
}

async fn hold_tickets_per_repo(store: &dyn Store, ws: WorkspaceId) {
    use maidan_types::UniqueAttributeKey::{LinearIssue, Repo};
    store
        .set_workspace_unique_thread_attributes(
            ws,
            maidan_types::WorkspaceUniqueThreadAttributes {
                unique: vec![vec![LinearIssue, Repo]],
            },
            maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(ws),
                actor_id: None,
                action: "workspace.unique_thread_attributes.set".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(ws.0),
                metadata: json!({}),
            },
        )
        .await
        .unwrap();
}

/// A follow-up round created with `supersedes` takes the ticket the round
/// before it holds, which closes as superseded in the same call. Without
/// `supersedes` the held ticket still answers 409.
#[tokio::test]
async fn a_round_created_with_supersedes_takes_the_ticket_and_closes_the_round_before_it() {
    let (base, store) = spawn().await;
    let a = room(store.as_ref(), "supersede-e2e").await;
    hold_tickets_per_repo(store.as_ref(), a.ws).await;
    let (status, old) = create(
        &base,
        &a.token,
        &a,
        json!({ "title": "round 4", "attributes": ticket() }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{old}");
    let old = ThreadId(old["id"].as_str().unwrap().parse().unwrap());

    let (status, held) = create(
        &base,
        &a.token,
        &a,
        json!({ "title": "round 5", "attributes": ticket() }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{held}");
    assert_eq!(held["holder_thread_id"], old.0.to_string());

    let (status, round) = create(
        &base,
        &a.token,
        &a,
        json!({ "title": "round 5", "attributes": ticket(), "supersedes": old.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{round}");
    let round = ThreadId(round["id"].as_str().unwrap().parse().unwrap());
    assert_eq!(
        store.get_thread(old).await.unwrap().state,
        ThreadState::Closed
    );
    assert_eq!(
        store.thread_unlanded_close(old).await.unwrap().as_deref(),
        Some("superseded")
    );
    assert_eq!(
        store.get_thread(round).await.unwrap().state,
        ThreadState::Open
    );
}

/// A `supersedes` naming another workspace's thread, a closed thread, or a
/// thread the caller may not close creates nothing and closes nothing.
#[tokio::test]
async fn a_supersedes_the_caller_may_not_act_on_creates_nothing() {
    let (base, store) = spawn().await;
    let a = room(store.as_ref(), "supersede-refused-a").await;
    let b = room(store.as_ref(), "supersede-refused-b").await;
    let stranger = token(store.as_ref(), a.ws, "stranger").await;
    let t_b = create_thread(&base, &b, "b's round").await;
    let t_closed = create_thread(&base, &a, "already closed").await;
    let (status, _) = close(&base, &a.token, t_closed, json!({ "reason": "abandoned" })).await;
    assert_eq!(status, StatusCode::OK);
    let t_mine = create_thread(&base, &a, "a's round").await;
    let before = store.list_threads(a.channel).await.unwrap().len();

    let (status, body) = create(
        &base,
        &a.token,
        &a,
        json!({ "title": "next", "supersedes": t_b.0 }),
    )
    .await;
    assert!(
        status == StatusCode::FORBIDDEN
            || status == StatusCode::NOT_FOUND
            || status == StatusCode::BAD_REQUEST,
        "{status} {body}"
    );
    let (status, body) = create(
        &base,
        &a.token,
        &a,
        json!({ "title": "next", "supersedes": t_closed.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = create(
        &base,
        &stranger,
        &a,
        json!({ "title": "next", "supersedes": t_mine.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    assert_eq!(store.list_threads(a.channel).await.unwrap().len(), before);
    assert_eq!(store.list_threads(b.channel).await.unwrap().len(), 1);
    assert_eq!(
        store.get_thread(t_b).await.unwrap().state,
        ThreadState::Open
    );
    assert_eq!(
        store.get_thread(t_mine).await.unwrap().state,
        ThreadState::Open
    );
}
