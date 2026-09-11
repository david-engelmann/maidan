//! Required-reviewers over HTTP (Cluster 375.3, Wave 2 #22). Auth ENABLED
//! (reviewer_id is a real member FK): set/get the requirement, name a reviewer,
//! submit a review as that reviewer, and watch review-status flip to met. The
//! close-gate itself is store-tested (Cluster 375.2 review_gate).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
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

#[tokio::test]
async fn required_reviewers_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = |handle: &'static str| {
        let store = store.clone();
        let ws = ws.id;
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
        }
    };
    let operator = member("op").await;
    let reviewer = member("reviewer").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();

    let caps = vec![
        capability::THREAD_TRANSITION.into(),
        capability::WORKSPACE_READ.into(),
    ];
    let op_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, operator.id, caps.clone()).await
    );
    let rev_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, reviewer.id, caps).await
    );

    let tid = thread.id.0;

    // Set the requirement (2 approvals), read it back.
    let set = client
        .put(format!("{base}/threads/{tid}/review-requirement"))
        .header("Authorization", &op_h)
        .json(&json!({ "required_count": 2 }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);
    let got: Value = client
        .get(format!("{base}/threads/{tid}/review-requirement"))
        .header("Authorization", &op_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["required_count"], 2);

    // Name the reviewer.
    let add = client
        .post(format!("{base}/threads/{tid}/reviewers"))
        .header("Authorization", &op_h)
        .json(&json!({ "member_id": reviewer.id.0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::NO_CONTENT);
    let reviewers: Value = client
        .get(format!("{base}/threads/{tid}/reviewers"))
        .header("Authorization", &op_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reviewers.as_array().unwrap().len(), 1);

    // Status: 0 of 2, not met.
    let status0: Value = client
        .get(format!("{base}/threads/{tid}/review-status"))
        .header("Authorization", &op_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status0["approvals"], 0);
    assert_eq!(status0["approvals_met"], false);

    // The reviewer submits an approval (as themselves).
    let review = client
        .post(format!("{base}/threads/{tid}/reviews"))
        .header("Authorization", &rev_h)
        .json(&json!({ "decision": "approve", "note": "lgtm" }))
        .send()
        .await
        .unwrap();
    assert_eq!(review.status(), StatusCode::OK);
    let reviews: Value = client
        .get(format!("{base}/threads/{tid}/reviews"))
        .header("Authorization", &op_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reviews.as_array().unwrap().len(), 1);

    // Status: 1 of 2 now (still not met — one reviewer, needs 2).
    let status1: Value = client
        .get(format!("{base}/threads/{tid}/review-status"))
        .header("Authorization", &op_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status1["approvals"], 1);
    assert_eq!(status1["approvals_met"], false);

    // A token lacking thread:transition can't name a reviewer → 403.
    let reader = format!(
        "Bearer {}",
        mint(
            store.as_ref(),
            ws.id,
            operator.id,
            vec![capability::WORKSPACE_READ.into()],
        )
        .await
    );
    let denied = client
        .post(format!("{base}/threads/{tid}/reviewers"))
        .header("Authorization", &reader)
        .json(&json!({ "member_id": reviewer.id.0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}
