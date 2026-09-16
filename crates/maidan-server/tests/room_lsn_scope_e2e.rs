//! Cluster 398.8: `Maidan-Room-LSN` is **this room's** head, not the instance's.
//!
//! The header answers "how far behind is my projector?", which only works if the
//! number is comparable to a `log_id` the client has seen — and a client only
//! ever sees its own workspace's events. Reporting the instance-wide head meant
//! a fully caught-up projector could never reach it, because the remaining gap
//! was other tenants' writes. It also told every tenant the instance's total
//! event volume, and rode outbound webhooks to third parties.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
    ROOM_LSN_HEADER,
};
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

/// Two tenants, very different volumes. Each must see only its own head.
#[tokio::test]
async fn the_header_reports_the_callers_room_not_the_instance() {
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

    // Auth ENABLED — the scope comes from the resolved bearer, so the bypass
    // path would not exercise this at all.
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::with_capacity(256)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    std::mem::forget(dir);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let mut tokens = Vec::new();
    for (name, threads) in [("quiet", 1usize), ("busy", 12usize)] {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: format!("{name}-m"),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "c".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        // Different volumes, so the two heads cannot coincide by accident.
        for _ in 0..threads {
            store
                .create_thread_with_event(NewThread {
                    channel_id: channel.id,
                    parent_thread_id: None,
                    title: Some("t".into()),
                })
                .await
                .unwrap();
        }
        tokens.push((ws.id, mint(store.as_ref(), ws.id, member.id).await));
    }

    let instance_head = store.max_event_id().await.unwrap();
    let mut seen = Vec::new();
    for (ws, token) in &tokens {
        let resp = client
            .get(format!("{base}/workspaces/{}", ws.0))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let lsn: i64 = resp
            .headers()
            .get(ROOM_LSN_HEADER)
            .expect("header present for an authenticated room")
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        let own_head = store
            .workspace_event_head(*ws)
            .await
            .unwrap()
            .map(|l| l.id)
            .unwrap_or(0);
        assert_eq!(lsn, own_head, "the header must be this room's head");
        seen.push(lsn);
    }

    // The quiet tenant must NOT be told the instance's volume.
    assert!(
        seen[0] < instance_head,
        "the quiet room's head ({}) should be below the instance head ({instance_head}) — \
         reporting the instance head is the leak and the unreachable target",
        seen[0]
    );
    assert_ne!(
        seen[0], seen[1],
        "two rooms at different volumes, two heads"
    );
}

/// An unauthenticated response carries no room, so it carries no header.
#[tokio::test]
async fn an_unauthenticated_response_carries_no_room_head() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::with_capacity(16)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    std::mem::forget(dir);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/workspaces/{}", uuid::Uuid::nil()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(
        resp.headers().get(ROOM_LSN_HEADER).is_none(),
        "a rejected request has no room and must not be told any head"
    );
}
