//! Cluster 385.4: Soundcheck pointer over HTTP. Auth ENABLED (recorded_by
//! is a real member FK). Require arms the gate; close 409s on amber / fail /
//! implementer pass; a soundcheck-skilled third party green pass lands.
//! MCP close uses the store FSM (P1.1d owns the MCP transition_thread twin).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_fsm::ThreadAction;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
    SOUNDCHECK_SKILL,
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

#[tokio::test]
async fn soundcheck_http_blocks_close_until_a_qualifying_green_pass() {
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
    let owner = member("owner").await;
    let assignee = member("assignee").await;
    let checker = member("soundcheck").await;
    let unskilled = member("unskilled").await;
    store
        .add_member_skill(checker.id, SOUNDCHECK_SKILL)
        .await
        .unwrap();
    store
        .add_member_skill(owner.id, SOUNDCHECK_SKILL)
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
            title: Some("land this".into()),
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .unwrap();
    store.assign_thread(thread.id, assignee.id).await.unwrap();

    let caps = vec![
        capability::THREAD_TRANSITION.into(),
        capability::WORKSPACE_READ.into(),
    ];
    let owner_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, owner.id, caps.clone()).await
    );
    let sc_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, checker.id, caps.clone()).await
    );
    let unskilled_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, unskilled.id, caps).await
    );
    let tid = thread.id.0;

    // Vacuous GET — no row, green, landable.
    let vacant: Value = client
        .get(format!("{base}/threads/{tid}/soundcheck"))
        .header("Authorization", &owner_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(vacant["required"], false);
    assert_eq!(vacant["landable"], true);
    assert_eq!(vacant["land"], "green");

    // Unskilled PUT is 400.
    let denied = client
        .put(format!("{base}/threads/{tid}/soundcheck"))
        .header("Authorization", &unskilled_h)
        .json(&json!({ "status": "pass" }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);

    // Require arms the gate.
    let req = client
        .put(format!("{base}/threads/{tid}/soundcheck/requirement"))
        .header("Authorization", &owner_h)
        .send()
        .await
        .unwrap();
    assert_eq!(req.status(), StatusCode::OK);
    let pending: Value = req.json().await.unwrap();
    assert_eq!(pending["required"], true);
    assert_eq!(pending["landable"], false);
    assert_eq!(pending["land"], "red");

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
        "pending require must refuse closed: {}",
        blocked.text().await.unwrap_or_default()
    );

    // Amber is flags-then-still-engages — not a land.
    let amber = client
        .put(format!("{base}/threads/{tid}/soundcheck"))
        .header("Authorization", &sc_h)
        .json(&json!({ "status": "pass", "land": "amber", "artifact_sha": "deadbeef" }))
        .send()
        .await
        .unwrap();
    assert_eq!(amber.status(), StatusCode::OK);
    let standing: Value = amber.json().await.unwrap();
    assert_eq!(standing["land"], "amber");
    assert_eq!(standing["landable"], false);
    assert_eq!(standing["pointer"]["kind"], "soundcheck");
    assert_eq!(standing["pointer"]["artifact_sha"], "deadbeef");
    let amber_close = client
        .post(format!("{base}/threads/{tid}"))
        .header("Authorization", &owner_h)
        .json(&json!({ "actor_id": owner.id.0, "action": "close" }))
        .send()
        .await
        .unwrap();
    assert_eq!(amber_close.status(), StatusCode::CONFLICT);

    // Implementer (owner) pass is stored but not a land.
    let self_pass = client
        .put(format!("{base}/threads/{tid}/soundcheck"))
        .header("Authorization", &owner_h)
        .json(&json!({ "status": "pass" }))
        .send()
        .await
        .unwrap();
    assert_eq!(self_pass.status(), StatusCode::OK);
    let self_standing: Value = self_pass.json().await.unwrap();
    assert_eq!(self_standing["landable"], false);
    let self_close = client
        .post(format!("{base}/threads/{tid}"))
        .header("Authorization", &owner_h)
        .json(&json!({ "actor_id": owner.id.0, "action": "close" }))
        .send()
        .await
        .unwrap();
    assert_eq!(self_close.status(), StatusCode::CONFLICT);

    // Qualifying green pass from the soundcheck agent lands.
    let green = client
        .put(format!("{base}/threads/{tid}/soundcheck"))
        .header("Authorization", &sc_h)
        .json(&json!({ "status": "pass" }))
        .send()
        .await
        .unwrap();
    assert_eq!(green.status(), StatusCode::OK);
    let landed: Value = green.json().await.unwrap();
    assert_eq!(landed["land"], "green");
    assert_eq!(landed["landable"], true);

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
async fn soundcheck_fail_is_red_and_mcp_standing_matches_the_store_gate() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let checker = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "soundcheck".into(),
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
        .add_member_skill(checker.id, SOUNDCHECK_SKILL)
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
    let owner_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, owner.id, caps.clone()).await
    );
    let sc_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, checker.id, caps).await
    );
    let tid = thread.id.0;

    let fail = client
        .put(format!("{base}/threads/{tid}/soundcheck"))
        .header("Authorization", &sc_h)
        .json(&json!({ "status": "fail", "land": "green" }))
        .send()
        .await
        .unwrap();
    assert_eq!(fail.status(), StatusCode::OK);
    let standing: Value = fail.json().await.unwrap();
    assert_eq!(standing["pointer"]["status"], "fail");
    assert_eq!(standing["pointer"]["land"], "red");
    assert_eq!(standing["land"], "red");
    assert_eq!(standing["landable"], false);

    store
        .transition_thread(thread.id, owner.id, ThreadAction::StartReview)
        .await
        .unwrap();
    let blocked = store
        .transition_thread(thread.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(blocked, Err(StoreError::Conflict(ref m)) if m.contains("soundcheck")),
        "fail must block the store FSM (MCP has no transition_thread here — P1.1d), got {blocked:?}"
    );

    let got: Value = client
        .get(format!("{base}/threads/{tid}/soundcheck"))
        .header("Authorization", &owner_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["landable"], false);

    let mcp: Value = client
        .post(format!("{base}/mcp"))
        .header("Authorization", &owner_h)
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "get_soundcheck", "arguments": { "thread_id": tid } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = mcp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["land"], "red");
    assert_eq!(parsed["landable"], false);
    assert_eq!(parsed["pointer"]["status"], "fail");
}
