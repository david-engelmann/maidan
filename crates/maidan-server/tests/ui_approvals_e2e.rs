//! The `/ui/api` approval-gate twins (Cluster 350.8): the `/ui` Approvals tab
//! lists pending gates and answers one over the session-proxy routes, which
//! reuse the Cluster-350.3 handlers under the `/ui` auth middleware. Driven here
//! with a bearer (the middleware accepts session OR bearer); the DOM + accept
//! flow is covered by the Playwright spec `ui-tests/tests/approvals.spec.ts`.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewApprovalGate, NewChannel, NewMember, NewThread,
    NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
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
    let mut state = AppState::new(
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
    state.subscribe_resume_secret = Some(Arc::from(&b"ui-approvals-e2e-secret-key-000001!"[..]));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn ui_api_lists_and_answers_an_approval_gate() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace { name: "ui".into() })
        .await
        .unwrap();
    let human = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "operator".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("deploy".into()),
        })
        .await
        .unwrap();
    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: human.id,
            prompt: "Ship it?".into(),
            schema: None,
        })
        .await
        .unwrap();
    let token = mint(store.as_ref(), ws.id, human.id).await;

    // The /ui Approvals tab lists pending gates through the read proxy.
    let list: Vec<Value> = client
        .get(format!("{base}/ui/api/workspaces/{}/approval-gates", ws.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["gate"]["id"], json!(gate.id.0.to_string()));
    let request_state = list[0]["request_state"].as_str().unwrap().to_string();

    // Answering over the write proxy resolves it (the same CAS-on-pending path).
    let answered = client
        .post(format!("{base}/ui/api/approval-gates/{}/answer", gate.id))
        .bearer_auth(&token)
        .json(&json!({ "request_state": request_state, "action": "accept" }))
        .send()
        .await
        .unwrap();
    assert_eq!(answered.status(), StatusCode::OK);
    let body: Value = answered.json().await.unwrap();
    assert_eq!(body["state"], json!("accepted"));

    // The resolved gate leaves the pending list.
    let after: Vec<Value> = client
        .get(format!("{base}/ui/api/workspaces/{}/approval-gates", ws.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(after.is_empty());
}
