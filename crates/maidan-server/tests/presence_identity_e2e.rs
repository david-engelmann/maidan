//! Presence and typing speak for a member, so a subscription may announce only
//! the member its token belongs to.
//!
//! The subscribe frame's `member_id` was taken as given. A token for one member
//! could show another as online or typing to their whole workspace, and stamp
//! any member's durable last-seen time — including another workspace's — which
//! presence-aware email reads as "active, don't email".

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::frame::coding::CloseCode, Message},
};

struct Harness {
    addr: SocketAddr,
    store: Arc<dyn Store>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Harness {
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
    let mut state = AppState::new(
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
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        addr,
        store,
        server,
        _dir: dir,
    }
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id
}

async fn token(store: &dyn Store, ws: WorkspaceId, member_id: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::EVENT_SUBSCRIBE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

/// Subscribe announcing `as_member`, and return the first frame back.
async fn subscribe_as(h: &Harness, ws: WorkspaceId, bearer: &str, as_member: MemberId) -> Message {
    let url = format!("ws://{}/ws/subscribe", h.addr);
    let (mut socket, _) = connect_async(url.into_client_request().unwrap())
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({
                "token": bearer,
                "filter": { "workspace_id": ws.0 },
                "member_id": as_member.0,
            })
            .to_string(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("a frame")
        .expect("open")
        .expect("ok")
}

fn is_policy_close(frame: &Message) -> bool {
    matches!(frame, Message::Close(Some(close)) if close.code == CloseCode::Policy)
}

#[tokio::test]
async fn a_subscription_announces_only_its_own_member() {
    let h = spawn().await;
    let alpha = workspace(h.store.as_ref(), "alpha").await;
    let bravo = workspace(h.store.as_ref(), "bravo").await;
    let me = member(h.store.as_ref(), alpha, "me").await;
    let colleague = member(h.store.as_ref(), alpha, "colleague").await;
    let stranger = member(h.store.as_ref(), bravo, "stranger").await;
    let bearer = token(h.store.as_ref(), alpha, me).await;

    let frame = subscribe_as(&h, alpha, &bearer, colleague).await;
    assert!(is_policy_close(&frame), "a colleague's presence: {frame:?}");

    let frame = subscribe_as(&h, alpha, &bearer, stranger).await;
    assert!(
        is_policy_close(&frame),
        "another workspace's member: {frame:?}"
    );
    assert_eq!(
        h.store.get_member_last_seen(stranger).await.unwrap(),
        None,
        "another tenant's member was never marked active"
    );
    assert_eq!(h.store.get_member_last_seen(colleague).await.unwrap(), None);

    let frame = subscribe_as(&h, alpha, &bearer, me).await;
    assert!(
        matches!(&frame, Message::Text(_)),
        "the caller's own presence is accepted: {frame:?}"
    );
    let mut seen = None;
    for _ in 0..50 {
        seen = h.store.get_member_last_seen(me).await.unwrap();
        if seen.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(seen.is_some(), "the caller is marked active");
    h.server.abort();
}
