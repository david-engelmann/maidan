//! An approval may be borrowed, but never by whoever did the work.
//!
//! A delegated token *is* the member it acts for, so every separation-of-duties
//! check that compared that member alone could be laundered: an orchestrator
//! claims a thread as the worker, then approves it with a token borrowed from
//! the reviewer. Each check now judges the delegate as well as the member it
//! acts as.
//!
//! The world: `orchestrator` holds grants from both `worker` and `reviewer`;
//! `outsider` holds a grant from `reviewer` and has never touched the work.
//! The outsider's borrowed approval counts — delegation stays useful — and the
//! orchestrator's does not.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use chrono::{Duration as ChronoDuration, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewApprovalGate, NewChannel, NewMember, NewThread,
    NewWorkspace, ThreadId, WorkspaceId, LAND_GATE_SKILL,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const WORK: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::THREAD_TRANSITION,
];

struct World {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    ws: WorkspaceId,
    channel: maidan_types::ChannelId,
    worker: MemberId,
    orchestrator: MemberId,
    outsider: MemberId,
    reviewer: MemberId,
    worker_tok: String,
    reviewer_tok: String,
    /// The orchestrator acting as the worker.
    orch_as_worker: String,
    /// The orchestrator acting as the reviewer.
    orch_as_reviewer: String,
    /// The outsider acting as the reviewer.
    outsider_as_reviewer: String,
    _dir: tempfile::TempDir,
}

async fn mint(store: &Arc<dyn Store>, ws: WorkspaceId, member: MemberId, caps: &[&str]) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn world() -> World {
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

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for handle in ["admin", "worker", "orchestrator", "outsider", "reviewer"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let [admin, worker, orchestrator, outsider, reviewer] = ids[..] else {
        unreachable!()
    };
    store
        .add_member_skill(reviewer, LAND_GATE_SKILL)
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

    let admin_tok = mint(&store, ws.id, admin, &[capability::TOKEN_ADMIN]).await;
    let worker_tok = mint(&store, ws.id, worker, WORK).await;
    let orch_tok = mint(&store, ws.id, orchestrator, WORK).await;
    let outsider_tok = mint(&store, ws.id, outsider, WORK).await;
    let reviewer_tok = mint(&store, ws.id, reviewer, WORK).await;

    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    // Approval gates sign their `request_state` with this secret.
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let borrow = |subject: MemberId, delegate: MemberId, delegate_tok: String| {
        let client = client.clone();
        let base = base.clone();
        let admin_tok = admin_tok.clone();
        async move {
            let grant: Value = client
                .post(format!("{base}/workspaces/{}/delegation-grants", ws.id.0))
                .bearer_auth(&admin_tok)
                .json(&json!({
                    "subject_id": subject.0,
                    "delegate_id": delegate.0,
                    "capabilities": WORK,
                    "purpose": "attestation test",
                    "expires_at": Utc::now() + ChronoDuration::hours(1),
                }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let exchanged: Value = client
                .post(format!("{base}/tokens/delegate"))
                .bearer_auth(&delegate_tok)
                .json(&json!({ "grant_id": grant["id"] }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            exchanged["token"]["secret"]
                .as_str()
                .unwrap_or_else(|| panic!("exchange failed: {exchanged}"))
                .to_string()
        }
    };
    let orch_as_worker = borrow(worker, orchestrator, orch_tok.clone()).await;
    let orch_as_reviewer = borrow(reviewer, orchestrator, orch_tok).await;
    let outsider_as_reviewer = borrow(reviewer, outsider, outsider_tok).await;

    World {
        base,
        client,
        store,
        ws: ws.id,
        channel: channel.id,
        worker,
        orchestrator,
        outsider,
        reviewer,
        worker_tok,
        reviewer_tok,
        orch_as_worker,
        orch_as_reviewer,
        outsider_as_reviewer,
        _dir: dir,
    }
}

impl World {
    /// A fresh thread the orchestrator claims *as the worker*.
    async fn thread_worked_by_orchestrator_as_worker(&self) -> ThreadId {
        let thread = self
            .store
            .create_thread(NewThread {
                channel_id: self.channel,
                parent_thread_id: None,
                title: Some("work".into()),
            })
            .await
            .unwrap();
        let claimed = self
            .client
            .post(format!(
                "{}/threads/{}/assignee/claim",
                self.base, thread.id.0
            ))
            .bearer_auth(&self.orch_as_worker)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            claimed.status(),
            StatusCode::OK,
            "{}",
            claimed.text().await.unwrap()
        );
        thread.id
    }

    async fn call(&self, method: reqwest::Method, token: &str, path: &str, body: Value) -> Value {
        let resp = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        assert!(status.is_success(), "{path}: {status} {body}");
        body
    }

    async fn approvals(&self, thread: ThreadId) -> i64 {
        let status = self
            .call(
                reqwest::Method::GET,
                &self.worker_tok,
                &format!("/threads/{}/review-status", thread.0),
                Value::Null,
            )
            .await;
        status["approvals"].as_i64().unwrap()
    }

    async fn approve(&self, token: &str, thread: ThreadId) -> Value {
        self.call(
            reqwest::Method::POST,
            token,
            &format!("/threads/{}/reviews", thread.0),
            json!({ "decision": "approve" }),
        )
        .await
    }

    async fn record_pass(&self, token: &str, thread: ThreadId) -> Value {
        self.call(
            reqwest::Method::PUT,
            token,
            &format!("/threads/{}/land-gate", thread.0),
            json!({ "status": "pass" }),
        )
        .await
    }

    /// Move the thread into review (a no-op once it is there) and try to close.
    async fn close(&self, thread: ThreadId) -> StatusCode {
        let transition = |action: &'static str| {
            self.client
                .post(format!("{}/threads/{}", self.base, thread.0))
                .bearer_auth(&self.worker_tok)
                .json(&json!({ "action": action }))
                .send()
        };
        let _ = transition("start_review").await.unwrap();
        transition("close").await.unwrap().status()
    }
}

#[tokio::test]
async fn a_delegate_that_did_the_work_cannot_approve_it_with_a_reviewers_token() {
    let w = world().await;
    let thread = w.thread_worked_by_orchestrator_as_worker().await;

    let review = w.approve(&w.orch_as_reviewer, thread).await;
    assert_eq!(review["reviewer_id"], json!(w.reviewer.0));
    assert_eq!(
        review["actor_id"],
        json!(w.orchestrator.0),
        "the review records who actually submitted it"
    );
    assert_eq!(
        w.approvals(thread).await,
        0,
        "it did the work; it cannot approve it"
    );

    // The same borrowed approval from someone who never touched the work counts.
    let review = w.approve(&w.outsider_as_reviewer, thread).await;
    assert_eq!(review["actor_id"], json!(w.outsider.0));
    assert_eq!(w.approvals(thread).await, 1);
}

/// The review requirement is what a close enforces, so the same exclusion has
/// to hold there — not only in the status a client reads.
#[tokio::test]
async fn a_close_does_not_count_a_delegates_approval_of_its_own_work() {
    let w = world().await;
    let thread = w.thread_worked_by_orchestrator_as_worker().await;
    w.call(
        reqwest::Method::PUT,
        &w.worker_tok,
        &format!("/threads/{}/review-requirement", thread.0),
        json!({ "required_count": 1 }),
    )
    .await;

    w.approve(&w.orch_as_reviewer, thread).await;
    assert_eq!(w.close(thread).await, StatusCode::CONFLICT);

    w.approve(&w.outsider_as_reviewer, thread).await;
    assert_eq!(w.close(thread).await, StatusCode::OK);
}

#[tokio::test]
async fn a_delegate_that_did_the_work_cannot_pass_it_through_the_land_gate() {
    let w = world().await;
    let thread = w.thread_worked_by_orchestrator_as_worker().await;
    w.call(
        reqwest::Method::PUT,
        &w.worker_tok,
        &format!("/threads/{}/land-gate/requirement", thread.0),
        json!({}),
    )
    .await;

    let standing = w.record_pass(&w.orch_as_reviewer, thread).await;
    assert_eq!(standing["landable"], false, "{standing}");
    assert_eq!(w.close(thread).await, StatusCode::CONFLICT);

    let standing = w.record_pass(&w.outsider_as_reviewer, thread).await;
    assert_eq!(standing["landable"], true, "{standing}");
    assert_eq!(w.close(thread).await, StatusCode::OK);
}

impl World {
    async fn open_gate(&self, requested_by: MemberId) -> (String, String) {
        let gate = self
            .store
            .create_approval_gate(&NewApprovalGate {
                workspace_id: self.ws,
                thread_id: None,
                requested_by,
                prompt: "ship it?".into(),
                schema: None,
            })
            .await
            .unwrap();
        let pending: Vec<Value> = self
            .client
            .get(format!(
                "{}/workspaces/{}/approval-gates",
                self.base, self.ws.0
            ))
            .bearer_auth(&self.reviewer_tok)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let entry = pending
            .iter()
            .find(|g| g["gate"]["id"] == json!(gate.id.0.to_string()))
            .unwrap();
        (
            gate.id.0.to_string(),
            entry["request_state"].as_str().unwrap().to_string(),
        )
    }

    async fn answer(
        &self,
        token: &str,
        gate: &(String, String),
        action: &str,
    ) -> reqwest::Response {
        self.client
            .post(format!("{}/approval-gates/{}/answer", self.base, gate.0))
            .bearer_auth(token)
            .json(&json!({ "request_state": gate.1, "action": action }))
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn no_one_accepts_their_own_approval_request() {
    let w = world().await;

    let gate = w.open_gate(w.worker).await;
    assert_eq!(
        w.answer(&w.worker_tok, &gate, "accept").await.status(),
        StatusCode::FORBIDDEN,
        "the requester cannot approve its own request"
    );
    assert_eq!(
        w.answer(&w.orch_as_worker, &gate, "accept").await.status(),
        StatusCode::FORBIDDEN,
        "nor can a delegate acting as the requester"
    );
    let accepted = w.answer(&w.reviewer_tok, &gate, "accept").await;
    assert_eq!(accepted.status(), StatusCode::OK);

    // Withdrawing your own request is not approving it.
    let withdrawn = w.open_gate(w.worker).await;
    assert_eq!(
        w.answer(&w.worker_tok, &withdrawn, "cancel").await.status(),
        StatusCode::OK
    );
}

/// A gate opened by a delegate for the worker records the delegate, and the
/// delegate cannot then accept it under a reviewer's borrowed token — while an
/// independent delegate for that reviewer can.
#[tokio::test]
async fn a_delegate_cannot_accept_a_request_it_made_under_another_identity() {
    let w = world().await;
    let requested: Value = w
        .client
        .post(format!("{}/mcp", w.base))
        .bearer_auth(&w.orch_as_worker)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "request_approval",
                "arguments": { "prompt": "ship it?" }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(requested["error"].is_null(), "{requested}");

    let pending: Vec<Value> = w
        .client
        .get(format!("{}/workspaces/{}/approval-gates", w.base, w.ws.0))
        .bearer_auth(&w.reviewer_tok)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0]["gate"]["requested_by"], json!(w.worker.0));
    assert_eq!(
        pending[0]["gate"]["requested_actor_id"],
        json!(w.orchestrator.0),
        "the gate records who actually asked"
    );
    let gate = (
        pending[0]["gate"]["id"].as_str().unwrap().to_string(),
        pending[0]["request_state"].as_str().unwrap().to_string(),
    );

    assert_eq!(
        w.answer(&w.orch_as_reviewer, &gate, "accept")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let accepted: Value = w
        .answer(&w.outsider_as_reviewer, &gate, "accept")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(accepted["state"], "accepted", "{accepted}");
    assert_eq!(accepted["resolved_by"], json!(w.reviewer.0));
    assert_eq!(accepted["resolved_actor_id"], json!(w.outsider.0));
}
