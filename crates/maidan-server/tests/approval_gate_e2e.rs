//! The held gate over HTTP (Cluster 350.3): a human lists pending approval
//! gates (each with a server-issued `request_state`) and answers one
//! accept/decline/cancel. Runs with auth ENABLED so `resolved_by` is a real
//! member and the HMAC `request_state` has a configured secret. Covers the CAS
//! no-op on a double-answer (silence is not consent), a tampered `request_state`
//! (403), and an unknown action (400).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewApprovalGate, NewMember, NewWorkspace, WorkspaceId,
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
    // The `request_state` HMAC is keyed on the server secret.
    state.subscribe_resume_secret = Some(Arc::from(&b"approval-gate-e2e-secret-key-0001!"[..]));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn list_and_answer_an_approval_gate() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "held".into(),
        })
        .await
        .unwrap();
    let human = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "oncall".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let write_token = mint(
        store.as_ref(),
        ws.id,
        human.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
    )
    .await;

    // An agent opened a gate (seeded via the store — the MCP request_approval
    // path is covered by the maidan-mcp inline test).
    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: None,
            requested_by: human.id,
            prompt: "Deploy v9 to prod?".into(),
            schema: None,
        })
        .await
        .unwrap();

    // The pending list carries the gate and its request_state.
    let list: Vec<Value> = client
        .get(format!("{base}/workspaces/{}/approval-gates", ws.id))
        .bearer_auth(&write_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["gate"]["id"], json!(gate.id.0.to_string()));
    assert_eq!(list[0]["gate"]["state"], json!("pending"));
    let request_state = list[0]["request_state"].as_str().unwrap().to_string();

    // A tampered request_state is refused (integrity of the untrusted round-trip).
    let bad = client
        .post(format!("{base}/approval-gates/{}/answer", gate.id))
        .bearer_auth(&write_token)
        .json(&json!({ "request_state": "deadbeef", "action": "accept" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::FORBIDDEN);

    // An unknown action is a bad request — never a silent accept.
    let bad_action = client
        .post(format!("{base}/approval-gates/{}/answer", gate.id))
        .bearer_auth(&write_token)
        .json(&json!({ "request_state": request_state, "action": "maybe" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_action.status(), StatusCode::BAD_REQUEST);

    // The human accepts, with structured content.
    let answered: Value = {
        let resp = client
            .post(format!("{base}/approval-gates/{}/answer", gate.id))
            .bearer_auth(&write_token)
            .json(&json!({
                "request_state": request_state,
                "action": "accept",
                "content": { "note": "ship it" }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        resp.json().await.unwrap()
    };
    assert_eq!(answered["state"], json!("accepted"));
    assert_eq!(answered["content"]["note"], json!("ship it"));
    assert_eq!(answered["resolved_by"], json!(human.id.0.to_string()));

    // Silence is not consent, and a second answer cannot flip it: the CAS on
    // `pending` finds nothing → 409.
    let again = client
        .post(format!("{base}/approval-gates/{}/answer", gate.id))
        .bearer_auth(&write_token)
        .json(&json!({ "request_state": request_state, "action": "decline" }))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::CONFLICT);

    // A resolved gate leaves the pending list.
    let list_after: Vec<Value> = client
        .get(format!("{base}/workspaces/{}/approval-gates", ws.id))
        .bearer_auth(&write_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list_after.is_empty());
}
