//! Cluster 383.3: a delivered `pi.review.result/1` with any `critical`
//! finding arms the Cluster-375 close-gate over HTTP. Auth ENABLED (real
//! tokens): the review-skilled producer PUTs the result; close 409s until a
//! human who is neither owner nor assignee approves.

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
    PI_REVIEW_RESULT_KIND, REVIEW_SKILL, WAITER_RESULT_SCHEMA,
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

fn critical_envelope() -> Value {
    json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": PI_REVIEW_RESULT_KIND,
        "status": "reviewed",
        "findings": [{ "severity": "critical" }],
    })
}

#[tokio::test]
async fn critical_review_result_blocks_close_until_a_human_approves() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = |handle: &'static str, kind: MemberKind| {
        let store = store.clone();
        let ws = ws.id;
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind,
                })
                .await
                .unwrap()
        }
    };
    let owner = member("owner", MemberKind::Human).await;
    let assignee = member("assignee", MemberKind::Agent).await;
    let reviewer = member("pi", MemberKind::Agent).await;
    let human = member("human", MemberKind::Human).await;
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
            title: Some("land this".into()),
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .unwrap();
    store.assign_thread(thread.id, assignee.id).await.unwrap();
    store
        .add_member_skill(reviewer.id, REVIEW_SKILL)
        .await
        .unwrap();

    let caps = vec![
        capability::THREAD_TRANSITION.into(),
        capability::WORKSPACE_READ.into(),
    ];
    let owner_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, owner.id, caps.clone()).await
    );
    let rev_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, reviewer.id, caps.clone()).await
    );
    let human_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, human.id, caps).await
    );
    let tid = thread.id.0;

    let set = client
        .put(format!("{base}/threads/{tid}/result"))
        .header("Authorization", &rev_h)
        .json(&json!({ "result": critical_envelope() }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);

    let status: Value = client
        .get(format!("{base}/threads/{tid}/review-status"))
        .header("Authorization", &owner_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["required_count"], 1);
    assert_eq!(status["approvals_met"], false);

    let reviews: Value = client
        .get(format!("{base}/threads/{tid}/reviews"))
        .header("Authorization", &owner_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reviews[0]["decision"], "request_changes");
    assert_eq!(reviews[0]["reviewer_id"], json!(reviewer.id.0));

    let start = client
        .post(format!("{base}/threads/{tid}"))
        .header("Authorization", &owner_h)
        .json(&json!({ "actor_id": owner.id.0, "action": "start_review" }))
        .send()
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::OK);

    let blocked = client
        .post(format!("{base}/threads/{tid}"))
        .header("Authorization", &owner_h)
        .json(&json!({ "actor_id": owner.id.0, "action": "close" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        blocked.status(),
        StatusCode::CONFLICT,
        "critical finding must refuse closed: {}",
        blocked.text().await.unwrap_or_default()
    );

    let approve = client
        .post(format!("{base}/threads/{tid}/reviews"))
        .header("Authorization", &human_h)
        .json(&json!({ "decision": "approve" }))
        .send()
        .await
        .unwrap();
    assert_eq!(approve.status(), StatusCode::OK);

    let closed = client
        .post(format!("{base}/threads/{tid}"))
        .header("Authorization", &owner_h)
        .json(&json!({ "actor_id": owner.id.0, "action": "close" }))
        .send()
        .await
        .unwrap();
    assert_eq!(closed.status(), StatusCode::OK);
    let body: Value = closed.json().await.unwrap();
    assert_eq!(body["state"], "closed");
}

#[tokio::test]
async fn a_warning_only_review_does_not_arm_the_close_gate() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let reviewer = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "pi".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    store
        .add_member_skill(reviewer.id, REVIEW_SKILL)
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
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .unwrap();

    let caps = vec![
        capability::THREAD_TRANSITION.into(),
        capability::WORKSPACE_READ.into(),
    ];
    let rev_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, reviewer.id, caps.clone()).await
    );
    let owner_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, owner.id, caps).await
    );
    let tid = thread.id.0;

    let set = client
        .put(format!("{base}/threads/{tid}/result"))
        .header("Authorization", &rev_h)
        .json(&json!({
            "result": {
                "schema": WAITER_RESULT_SCHEMA,
                "result_kind": PI_REVIEW_RESULT_KIND,
                "status": "reviewed",
                "findings": [{ "severity": "warning" }],
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);

    let status: Value = client
        .get(format!("{base}/threads/{tid}/review-status"))
        .header("Authorization", &owner_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["required_count"], 0);
    assert_eq!(status["approvals_met"], true);
}
