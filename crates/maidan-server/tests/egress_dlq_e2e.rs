//! Operator DLQ for the durable projector egress (Cluster 377.4). Auth ENABLED —
//! `token:admin` is the whole point of the surface, so a bypass run would prove
//! nothing. Covers the loop an operator actually walks: a delivery dead-letters,
//! it shows up with the surface's own error, a requeue makes it deliverable
//! again, and a workspace-scoped token cannot see any of it.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EgressTarget, MemberId, MemberKind, NewApiToken, NewChannel, NewEgressOutbox, NewMember,
    NewThread, NewWorkspace, ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::Value;
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

/// A dead-lettered delivery, the way the worker leaves one: enqueued, claimed
/// (which bumps `attempts`), then failed with no retry.
async fn dead_delivery(store: &dyn Store, ws: WorkspaceId, thread: ThreadId, error: &str) {
    let id = store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: ws,
            thread_id: thread,
            source_log_id: 1,
            target: EgressTarget::Github {
                repo: "acme/widgets".into(),
                issue_number: 42,
            },
            body: "the review".into(),
        })
        .await
        .unwrap()
        .expect("inserted");
    store
        .claim_next_due_egress(chrono::Utc::now(), 300)
        .await
        .unwrap()
        .expect("claimed");
    store.mark_egress_failed(id, error, None).await.unwrap();
}

#[tokio::test]
async fn operator_lists_and_requeues_a_dead_projector_delivery() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let op = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
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
    let admin = mint(
        store.as_ref(),
        ws.id,
        op.id,
        vec![capability::TOKEN_ADMIN.into()],
    )
    .await;

    // Empty to start.
    let list = |token: String| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .get(format!("{base}/operator/egress/dead"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
        }
    };
    let resp = list(admin.clone()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.json::<Vec<Value>>().await.unwrap().len(), 0);

    dead_delivery(
        store.as_ref(),
        ws.id,
        thread.id,
        "github api error: status 404",
    )
    .await;

    // The operator sees what failed, where it was going, and why.
    let rows = list(admin.clone())
        .await
        .json::<Vec<Value>>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["surface"], "github");
    assert_eq!(rows[0]["selector"], "acme/widgets#42");
    assert_eq!(rows[0]["thread_id"], serde_json::json!(thread.id.0));
    assert_eq!(rows[0]["attempts"], 1);
    assert_eq!(rows[0]["last_error"], "github api error: status 404");
    let id = rows[0]["id"].as_str().unwrap().to_string();

    // Requeue makes it claimable again and empties the DLQ.
    let requeue = |id: &str, token: String| {
        let client = client.clone();
        let url = format!("{base}/operator/egress/dead/{id}/requeue");
        async move { client.post(url).bearer_auth(token).send().await.unwrap() }
    };
    assert_eq!(
        requeue(&id, admin.clone()).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        list(admin.clone())
            .await
            .json::<Vec<Value>>()
            .await
            .unwrap()
            .len(),
        0
    );
    let reclaimed = store
        .claim_next_due_egress(chrono::Utc::now(), 300)
        .await
        .unwrap()
        .expect("requeued is claimable");
    assert_eq!(
        reclaimed.attempts, 1,
        "requeue reset attempts (this claim -> 1)"
    );

    // Requeueing something that isn't a dead entry is a 404, not a silent no-op.
    assert_eq!(
        requeue(&id, admin.clone()).await.status(),
        StatusCode::NOT_FOUND,
        "already requeued"
    );
    assert_eq!(
        requeue(&uuid::Uuid::new_v4().to_string(), admin.clone())
            .await
            .status(),
        StatusCode::NOT_FOUND,
        "unknown id"
    );

    // The queue is cross-workspace, so a workspace-scoped token cannot read it.
    let member_token = mint(
        store.as_ref(),
        ws.id,
        op.id,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;
    assert_eq!(
        list(member_token.clone()).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        requeue(&id, member_token).await.status(),
        StatusCode::FORBIDDEN
    );
}
