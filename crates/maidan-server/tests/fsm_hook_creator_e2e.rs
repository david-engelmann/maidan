//! SEC-013 and SEC-017: an FSM hook runs as the token that registered it.
//!
//! The hook under test is the escalation the audit found: on every move into
//! `in_review`, run `clear_review_requirement` (a `channel:admin` tool), so a
//! worker's own work skips the review it needs. A hook used to run it with the
//! server's authority. Now:
//!
//! - a full admin's hook still fires and clears the requirement;
//! - a token without the tool's capability cannot register the hook, and a
//!   hook made for it anyway is refused when it fires;
//! - a hook whose creator's token is revoked, no longer holds
//!   `workspace:admin`, belongs to another workspace, or is missing, is
//!   skipped, with an `fsm_hook.skipped` row in the workspace's audit trail.
//!
//! Each case has its own workspace, since a hook matches every transition in
//! its workspace.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{expand_set, hash_secret, TokenSecret, AGENT_WORKER, HUMAN_ADMIN};
use maidan_bus::InMemoryBus;
use maidan_server::{fsm_hook_worker::FsmHookWorker, router, AppState};
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApiTokenId, MemberId, MemberKind, NewApiToken, NewChannel, NewFsmHook, NewMember, NewThread,
    NewWorkspace, SlashHandlerKind, ThreadId, ThreadState, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

struct Harness {
    base: String,
    store: Arc<dyn Store>,
    client: reqwest::Client,
    worker: Option<FsmHookWorker>,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn shutdown(mut self) {
        if let Some(worker) = self.worker.take() {
            worker.shutdown().await;
        }
    }
}

async fn spawn() -> Harness {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(8)
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
    let mut state = AppState::for_tests(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(256)),
        search,
    );
    state.auth_disabled = false;
    state.test_identity_header = false;
    let worker = FsmHookWorker::spawn(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        store,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        worker: Some(worker),
        _dir: dir,
    }
}

struct Minted {
    secret: String,
    id: ApiTokenId,
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

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> Minted {
    let secret = TokenSecret::generate();
    let token = store
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
    Minted {
        secret: secret.as_str().to_string(),
        id: token.id,
    }
}

/// One workspace with a worker who does the work, and a thread that needs
/// two approvals before it may close.
struct Case {
    ws: WorkspaceId,
    worker: MemberId,
    worker_token: Minted,
    thread: ThreadId,
}

async fn case(h: &Harness, name: &str) -> Case {
    let store = h.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let worker = member(store, ws, &format!("{name}-worker")).await;
    let worker_token = mint(store, ws, worker, expand_set(AGENT_WORKER).unwrap()).await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
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
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id;
    store.set_review_requirement(thread, 2).await.unwrap();
    Case {
        ws,
        worker,
        worker_token,
        thread,
    }
}

fn hook_body() -> Value {
    json!({
        "label": "strip review",
        "to_state": "in_review",
        "handler_kind": "mcp_tool",
        "handler_target": "clear_review_requirement",
    })
}

impl Harness {
    async fn register_rest(&self, ws: WorkspaceId, bearer: &str) -> (StatusCode, Value) {
        let resp = self
            .client
            .post(format!("{}/workspaces/{}/fsm-hooks", self.base, ws.0))
            .bearer_auth(bearer)
            .json(&hook_body())
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn register_mcp(&self, ws: WorkspaceId, bearer: &str) -> Value {
        let mut arguments = hook_body();
        arguments["workspace_id"] = json!(ws.0);
        self.client
            .post(format!("{}/mcp", self.base))
            .bearer_auth(bearer)
            .header("accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": "register_fsm_hook", "arguments": arguments},
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap_or(Value::Null)
    }

    /// A hook as a token would have registered it, written straight to the
    /// store: how a hook made before a rule, or for a token that lost
    /// authority since, looks at fire time.
    async fn stored_hook(&self, ws: WorkspaceId, creator: Option<ApiTokenId>) {
        self.store
            .create_fsm_hook(NewFsmHook {
                workspace_id: ws,
                label: Some("strip review".into()),
                from_state: None,
                to_state: Some(ThreadState::InReview),
                handler_kind: SlashHandlerKind::McpTool,
                handler_target: "clear_review_requirement".into(),
                secret_ciphertext: String::new(),
                created_by_token: creator,
            })
            .await
            .unwrap();
    }

    /// The worker posts its result and moves the thread into review, which
    /// fires every `in_review` hook in the workspace.
    async fn submit_for_review(&self, c: &Case) {
        let lease = self
            .store
            .claim_lease_for(c.thread, c.worker)
            .await
            .unwrap();
        let resp = self
            .client
            .put(format!("{}/threads/{}/result", self.base, c.thread.0))
            .bearer_auth(&c.worker_token.secret)
            .json(&json!({"result": {"ok": true}, "claim_lease_id": lease.0}))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "result: {}", resp.status());
        let resp = self
            .client
            .post(format!("{}/threads/{}", self.base, c.thread.0))
            .bearer_auth(&c.worker_token.secret)
            .json(&json!({"action": "start_review"}))
            .send()
            .await
            .unwrap();
        assert!(
            resp.status().is_success(),
            "start_review: {}",
            resp.status()
        );
    }

    async fn requirement_cleared(&self, c: &Case) -> bool {
        self.store
            .get_review_requirement(c.thread)
            .await
            .unwrap()
            .is_none()
    }

    /// Poll until the requirement is cleared, or give up after `within`.
    async fn cleared_within(&self, c: &Case, within: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        while tokio::time::Instant::now() < deadline {
            if self.requirement_cleared(c).await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        self.requirement_cleared(c).await
    }

    /// The reason recorded for a skipped hook in `ws`, waiting up to five
    /// seconds for the dispatch to reach it.
    async fn skip_reason(&self, ws: WorkspaceId) -> Option<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let rows = self.store.list_audit_for_workspace(ws, 50).await.unwrap();
            if let Some(row) = rows.iter().find(|row| row.action == "fsm_hook.skipped") {
                return row.metadata["reason"].as_str().map(str::to_string);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_admins_hook_still_fires_as_its_creator() {
    let h = spawn().await;
    let c = case(&h, "control").await;
    let admin = member(h.store.as_ref(), c.ws, "admin").await;
    let admin_token = mint(
        h.store.as_ref(),
        c.ws,
        admin,
        expand_set(HUMAN_ADMIN).unwrap(),
    )
    .await;

    let (status, body) = h.register_rest(c.ws, &admin_token.secret).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    h.submit_for_review(&c).await;
    assert!(
        h.cleared_within(&c, Duration::from_secs(5)).await,
        "the admin's hook did not fire"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_narrow_admin_is_refused_the_hook_at_registration_and_when_it_fires() {
    let h = spawn().await;
    let c = case(&h, "narrow").await;
    // Every registration capability, but not the channel:admin the tool
    // needs.
    let narrow = member(h.store.as_ref(), c.ws, "narrow").await;
    let narrow_token = mint(
        h.store.as_ref(),
        c.ws,
        narrow,
        vec![
            "workspace:admin".into(),
            "workspace:write".into(),
            "workspace:read".into(),
        ],
    )
    .await;

    let (status, body) = h.register_rest(c.ws, &narrow_token.secret).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "REST: {body}");
    let body = h.register_mcp(c.ws, &narrow_token.secret).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("channel:admin")),
        "MCP: {body}"
    );
    assert!(h.store.list_fsm_hooks(c.ws).await.unwrap().is_empty());

    // Made for it anyway, the hook runs the tool as the narrow admin, and the
    // tool refuses.
    h.stored_hook(c.ws, Some(narrow_token.id)).await;
    h.submit_for_review(&c).await;
    assert!(
        !h.cleared_within(&c, Duration::from_secs(2)).await,
        "the hook cleared a requirement its creator may not clear"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_creators_hook_is_skipped_and_recorded() {
    let h = spawn().await;
    let c = case(&h, "revoked").await;
    let admin = member(h.store.as_ref(), c.ws, "leaver").await;
    let admin_token = mint(
        h.store.as_ref(),
        c.ws,
        admin,
        expand_set(HUMAN_ADMIN).unwrap(),
    )
    .await;

    let (status, body) = h.register_rest(c.ws, &admin_token.secret).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    h.store.revoke_api_token(admin_token.id).await.unwrap();

    h.submit_for_review(&c).await;
    assert_eq!(
        h.skip_reason(c.ws).await.as_deref(),
        Some("creator token no longer valid")
    );
    assert!(
        !h.requirement_cleared(&c).await,
        "a revoked creator's hook still fired"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hook_whose_creator_lacks_workspace_admin_or_is_missing_or_foreign_is_skipped() {
    let h = spawn().await;
    let store = h.store.as_ref();

    // A worker's hook, as one registered before registration needed
    // workspace:admin would be.
    let by_worker = case(&h, "by-worker").await;
    h.stored_hook(by_worker.ws, Some(by_worker.worker_token.id))
        .await;

    // A hook with no creator: registered before creators were recorded.
    let unowned = case(&h, "unowned").await;
    h.stored_hook(unowned.ws, None).await;

    // A hook naming an admin token of another workspace.
    let foreign = case(&h, "foreign").await;
    let elsewhere = case(&h, "elsewhere").await;
    let other_admin = member(store, elsewhere.ws, "other-admin").await;
    let other_token = mint(
        store,
        elsewhere.ws,
        other_admin,
        expand_set(HUMAN_ADMIN).unwrap(),
    )
    .await;
    h.stored_hook(foreign.ws, Some(other_token.id)).await;

    for (c, reason) in [
        (&by_worker, "creator no longer holds workspace:admin"),
        (&unowned, "registered without a token"),
        (&foreign, "creator token is for another workspace"),
    ] {
        h.submit_for_review(c).await;
        assert_eq!(h.skip_reason(c.ws).await.as_deref(), Some(reason));
        assert!(!h.requirement_cleared(c).await, "{reason}: the hook fired");
    }
    h.shutdown().await;
}
