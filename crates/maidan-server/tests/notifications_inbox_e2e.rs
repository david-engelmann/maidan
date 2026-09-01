//! Cluster 239: the REST unified inbox — list / unread-count / mark-read / read-all
//! over `/members/:id/notifications`. Auth ENABLED with a minted bearer token (the
//! act-as-any orchestrator model; the self-only session guard is unit-tested in
//! routes::ensure_acting_member).

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
    EventKind, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewNotification,
    NewThread, NewWorkspace, WorkspaceId,
};
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
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn inbox_list_count_mark_and_read_all() {
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
    let bus = Arc::new(InMemoryBus::with_capacity(64));

    let ws = store
        .create_workspace(NewWorkspace { name: "n".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "me".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
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
        })
        .await
        .unwrap();
    // Seed three notifications for the member.
    let mut ids = Vec::new();
    for log_id in 1..=3 {
        let n = store
            .create_notification(NewNotification {
                workspace_id: ws.id,
                member_id: member.id,
                kind: EventKind::MentionRecorded,
                source_log_id: log_id,
                channel_id: Some(channel.id),
                thread_id: Some(thread.id),
                message_id: None,
                actor_id: None,
            })
            .await
            .unwrap();
        ids.push(n.id);
    }
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

    // List returns all three, newest first (the highest source_log_id).
    let list: Value = client
        .get(format!("{base}/members/{mid}/notifications"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 3);

    // Unread count is 3.
    let count: Value = client
        .get(format!("{base}/members/{mid}/notifications/unread-count"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(count["count"], 3);

    // Mark one read → count drops to 2.
    let marked = client
        .post(format!(
            "{base}/members/{mid}/notifications/{}/read",
            ids[0].0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(marked.status(), StatusCode::OK);
    let marked_body: Value = marked.json().await.unwrap();
    assert_eq!(marked_body["count"], 2);

    // unread_only now returns 2.
    let unread: Value = client
        .get(format!(
            "{base}/members/{mid}/notifications?unread_only=true"
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unread.as_array().unwrap().len(), 2);

    // Marking an unknown notification id → 404.
    let missing = client
        .post(format!(
            "{base}/members/{mid}/notifications/{}/read",
            uuid::Uuid::new_v4()
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    // Read-all clears the rest.
    let all: Value = client
        .post(format!("{base}/members/{mid}/notifications/read-all"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all["cleared"], 2);
    let after: Value = client
        .get(format!("{base}/members/{mid}/notifications/unread-count"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after["count"], 0);
}
