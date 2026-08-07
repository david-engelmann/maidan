//! Cluster 171: thread task assignment / handoff over REST.
//!
//! Runs with auth ENABLED. Proves assign/handoff, the atomic compare-and-set
//! claim (including a concurrent race → exactly one winner), unassign, event
//! emission, and RBAC denial for a non-member of a private channel.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Ctx {
    addr: SocketAddr,
    _server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

impl Ctx {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }
}

/// A full-capability token (incl. `thread:transition`) for `member`.
async fn mint(store: &dyn Store, ws: maidan_types::WorkspaceId, member: MemberId) -> String {
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
                capability::MESSAGE_POST.into(),
                capability::THREAD_TRANSITION.into(),
                capability::CHANNEL_ADMIN.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

/// A read/write token WITHOUT `thread:transition` (for RBAC/deny checks).
async fn mint_no_assign(
    store: &dyn Store,
    ws: maidan_types::WorkspaceId,
    member: MemberId,
) -> String {
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
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn spawn() -> Ctx {
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
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ctx {
        addr,
        _server: server,
        client: reqwest::Client::new(),
        store,
        _dir: dir,
    }
}

fn auth(t: &str) -> String {
    format!("Bearer {t}")
}

#[tokio::test]
async fn assign_claim_unassign_and_emit_events_over_rest() {
    let ctx = spawn().await;
    let base = ctx.base();

    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let mk = |h: &'static str| {
        let store = ctx.store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: h.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
        }
    };
    let alice = mk("alice").await;
    let bob = mk("bob").await;
    let tok = mint(ctx.store.as_ref(), ws.id, alice.id).await;

    // Public channel + thread.
    let ch: Value = ctx
        .client
        .post(format!("{base}/workspaces/{}/channels", ws.id.0))
        .header("Authorization", auth(&tok))
        .json(&json!({"name": "work"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();
    let th: Value = ctx
        .client
        .post(format!("{base}/channels/{cid}/threads"))
        .header("Authorization", auth(&tok))
        .json(&json!({"title": "task"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();
    assert!(th["assignee_id"].is_null(), "new thread is unassigned");

    // Assign to bob.
    let assigned: Value = ctx
        .client
        .put(format!("{base}/threads/{tid}/assignee"))
        .header("Authorization", auth(&tok))
        .json(&json!({"actor_id": alice.id.0, "assignee_id": bob.id.0}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        assigned["assignee_id"].as_str(),
        Some(bob.id.0.to_string().as_str())
    );

    // GET reflects it.
    let got: Value = ctx
        .client
        .get(format!("{base}/threads/{tid}"))
        .header("Authorization", auth(&tok))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        got["assignee_id"].as_str(),
        Some(bob.id.0.to_string().as_str())
    );

    // Claim on an already-assigned thread → claimed:false, assignee unchanged.
    let claim_busy: Value = ctx
        .client
        .post(format!("{base}/threads/{tid}/assignee/claim"))
        .header("Authorization", auth(&tok))
        .json(&json!({"member_id": alice.id.0}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(claim_busy["claimed"], json!(false));
    assert_eq!(
        claim_busy["thread"]["assignee_id"].as_str(),
        Some(bob.id.0.to_string().as_str()),
        "a failed claim must not steal the assignment"
    );

    // Unassign, then a claim succeeds.
    let cleared: Value = ctx
        .client
        .delete(format!("{base}/threads/{tid}/assignee"))
        .header("Authorization", auth(&tok))
        .json(&json!({"actor_id": alice.id.0}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(cleared["assignee_id"].is_null());

    let claim_ok: Value = ctx
        .client
        .post(format!("{base}/threads/{tid}/assignee/claim"))
        .header("Authorization", auth(&tok))
        .json(&json!({"member_id": alice.id.0}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(claim_ok["claimed"], json!(true));
    assert_eq!(
        claim_ok["thread"]["assignee_id"].as_str(),
        Some(alice.id.0.to_string().as_str())
    );

    // Events: assign + unassign + successful claim each emit a
    // `thread_assignment_changed` (the failed claim does not).
    let events: Value = ctx
        .client
        .get(format!(
            "{base}/workspaces/{}/events?after_id=0&limit=100",
            ws.id.0
        ))
        .header("Authorization", auth(&tok))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let arr = events.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let assignment_events = arr
        .iter()
        .filter(|e| e["kind"] == json!("thread_assignment_changed"))
        .count();
    assert_eq!(
        assignment_events, 3,
        "assign + unassign + 1 successful claim = 3 events (failed claim emits none)"
    );
}

#[tokio::test]
async fn concurrent_claims_have_exactly_one_winner() {
    let ctx = spawn().await;
    let base = ctx.base();

    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "race".into(),
        })
        .await
        .unwrap();
    let alice = ctx
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let bob = ctx
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let tok = mint(ctx.store.as_ref(), ws.id, alice.id).await;

    let ch: Value = ctx
        .client
        .post(format!("{base}/workspaces/{}/channels", ws.id.0))
        .header("Authorization", auth(&tok))
        .json(&json!({"name": "work"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();
    let th: Value = ctx
        .client
        .post(format!("{base}/channels/{cid}/threads"))
        .header("Authorization", auth(&tok))
        .json(&json!({"title": "contended"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();

    // Two members race to claim the same unassigned thread.
    let claim = |member: MemberId| {
        let client = ctx.client.clone();
        let url = format!("{base}/threads/{tid}/assignee/claim");
        let tok = tok.clone();
        async move {
            client
                .post(url)
                .header("Authorization", auth(&tok))
                .json(&json!({"member_id": member.0}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let (a, b) = tokio::join!(claim(alice.id), claim(bob.id));
    let wins = [&a, &b]
        .iter()
        .filter(|r| r["claimed"] == json!(true))
        .count();
    assert_eq!(
        wins, 1,
        "exactly one concurrent claim wins the compare-and-set"
    );
}

#[tokio::test]
async fn non_member_is_denied_claim_in_private_channel() {
    let ctx = spawn().await;
    let base = ctx.base();

    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let alice = ctx
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let mallory = ctx
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "mallory".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let alice_tok = mint(ctx.store.as_ref(), ws.id, alice.id).await;
    let mallory_tok = mint_no_assign(ctx.store.as_ref(), ws.id, mallory.id).await;

    // Alice creates a PRIVATE channel (auto-added) + a thread.
    let ch: Value = ctx
        .client
        .post(format!("{base}/workspaces/{}/channels", ws.id.0))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"name": "secret", "private": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();
    let th: Value = ctx
        .client
        .post(format!("{base}/channels/{cid}/threads"))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"title": "plans"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();

    // Mallory (not a channel member) is denied the claim despite holding
    // `thread:transition` — RBAC via ensure_channel_access.
    let resp = ctx
        .client
        .post(format!("{base}/threads/{tid}/assignee/claim"))
        .header("Authorization", auth(&mallory_tok))
        .json(&json!({"member_id": mallory.id.0}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
