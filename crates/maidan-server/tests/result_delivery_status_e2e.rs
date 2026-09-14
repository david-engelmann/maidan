//! Cluster 379.5: delivery-status REST + replay + audit-per-attempt.
//!
//! Auth ENABLED (real minted token) so `ensure_acting_member` is not in play
//! and `workspace:write` is actually exercised. The router is seeded via
//! `route_event` (no worker-loop timing); `sweep_once` is awaited directly
//! for the send-attempt audit.

use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc, Mutex,
    },
};

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{
    egress_worker, github::GithubSender, notification_router, router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    status, EgressSurface, Event, ExternalRef, MemberId, MemberKind, NewApiToken, NewChannel,
    NewEgressTarget, NewMember, NewThread, NewWorkspace, ResultDelivery, ResultDeliveryId,
    ThreadId, WorkspaceId, WAITER_RESULT_SCHEMA,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct RecordingGithub {
    next_id: AtomicI64,
    posts: Mutex<Vec<String>>,
}

impl RecordingGithub {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            next_id: AtomicI64::new(1),
            posts: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl GithubSender for RecordingGithub {
    async fn post_comment(
        &self,
        _repo: &str,
        _issue_number: i64,
        text: &str,
    ) -> Result<Option<ExternalRef>, maidan_server::github::GithubError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.posts.lock().unwrap().push(text.into());
        Ok(Some(ExternalRef::Github {
            repo: "acme/widgets".into(),
            comment_id: id,
        }))
    }

    async fn update_comment(
        &self,
        _repo: &str,
        _comment_id: i64,
        text: &str,
    ) -> Result<(), maidan_server::github::GithubError> {
        self.posts.lock().unwrap().push(text.into());
        Ok(())
    }

    async fn list_issue_comments(
        &self,
        _repo: &str,
        _issue_number: i64,
    ) -> Result<Vec<maidan_server::github::GithubIssueComment>, maidan_server::github::GithubError>
    {
        Ok(vec![])
    }

    async fn create_review(
        &self,
        _repo: &str,
        _pull_number: i64,
        _commit_id: &str,
        _comments: &[maidan_types::GithubReviewComment],
    ) -> Result<(), maidan_server::github::GithubError> {
        Ok(())
    }
}

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
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

fn envelope(status: &str, deliver_to: Value) -> Value {
    json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": "example.review.result/1",
        "status": status,
        "deliver_to": deliver_to,
        "rendered": "## Findings",
        "summary": "3 findings",
        "view_url": "https://producer.example.test/r/1",
        "pr": "acme/widgets#7",
    })
}

struct Harness {
    store: Arc<dyn Store>,
    state: AppState,
    base: String,
    bearer: String,
    workspace_id: WorkspaceId,
    channel_id: maidan_types::ChannelId,
    thread_id: ThreadId,
    member_id: MemberId,
}

async fn harness(name: &str) -> Harness {
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
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
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
            title: Some(name.into()),
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let mut state = AppState::new(
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
    state.attach_github_sender(RecordingGithub::new());
    let app = router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        store,
        state,
        base: format!("http://{addr}"),
        bearer: format!("Bearer {tok}"),
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        member_id: member.id,
    }
}

async fn route(h: &Harness, result: &Value, log_id: i64) {
    h.store
        .set_thread_result(h.thread_id, h.member_id, result)
        .await
        .unwrap();
    notification_router::route_event(
        &h.state,
        log_id,
        &Event::ThreadResultSet {
            occurred_at: Utc::now(),
            workspace_id: h.workspace_id,
            channel_id: h.channel_id,
            thread_id: h.thread_id,
            produced_by: h.member_id,
        },
    )
    .await
    .unwrap();
}

async fn list(h: &Harness) -> (StatusCode, Value) {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/threads/{}/deliveries", h.base, h.thread_id.0))
        .header("Authorization", &h.bearer)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.json().await.unwrap_or(json!(null));
    (status, body)
}

async fn replay(h: &Harness, did: ResultDeliveryId) -> (StatusCode, Value) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/threads/{}/deliveries/{}/replay",
            h.base, h.thread_id.0, did.0
        ))
        .header("Authorization", &h.bearer)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.json().await.unwrap_or(json!(null));
    (status, body)
}

#[tokio::test]
async fn list_replay_and_audit_over_http() {
    let h = harness("status").await;

    let (code, body) = list(&h).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body, json!([]), "no result yet → delivered nowhere");

    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: h.workspace_id,
            surface: EgressSurface::Github,
            selector: "acme/widgets".into(),
        })
        .await
        .unwrap();
    route(
        &h,
        &envelope(
            "reviewed",
            json!([{ "surface": "github", "repo": "acme/widgets", "pr": 7 }]),
        ),
        1,
    )
    .await;

    let (code, body) = list(&h).await;
    assert_eq!(code, StatusCode::OK);
    let rows: Vec<ResultDelivery> = serde_json::from_value(body).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, status::PENDING);
    assert_eq!(rows[0].surface, "github");
    let did = rows[0].id;

    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!(stats.sent, 1);
    let (code, body) = list(&h).await;
    assert_eq!(code, StatusCode::OK);
    let rows: Vec<ResultDelivery> = serde_json::from_value(body).unwrap();
    assert_eq!(rows[0].status, status::DELIVERED);
    assert!(rows[0].external_ref.is_some());

    let audit = h.store.list_audit(20).await.unwrap();
    assert!(
        audit.iter().any(|e| e.action == "result_delivery.attempt"
            && e.metadata["outcome"] == json!("sent")
            && e.actor_id.is_none()),
        "the worker send is audited, actor_id None: {audit:?}"
    );

    let (code, body) = replay(&h, did).await;
    assert_eq!(code, StatusCode::OK);
    let replayed: ResultDelivery = serde_json::from_value(body).unwrap();
    assert_eq!(replayed.status, status::PENDING);
    assert_eq!(
        replayed.external_ref, rows[0].external_ref,
        "replay keeps the handle so the next send updates in place"
    );
    assert_eq!(
        replayed.armed_revision, rows[0].armed_revision,
        "replay must not bump armed_revision"
    );

    let audit = h.store.list_audit(20).await.unwrap();
    assert!(
        audit.iter().any(|e| e.action == "result_delivery.replay"
            && e.actor_id == Some(h.member_id)
            && e.target_id == Some(did.0)),
        "the operator replay is audited as the minted member: {audit:?}"
    );

    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!(stats.sent, 1, "replay send updates in place");
    let (code, body) = list(&h).await;
    assert_eq!(code, StatusCode::OK);
    let rows: Vec<ResultDelivery> = serde_json::from_value(body).unwrap();
    assert_eq!(rows[0].status, status::DELIVERED);
}

#[tokio::test]
async fn unblessed_replay_stays_skipped_and_unroutable_is_400() {
    let h = harness("skip").await;
    route(
        &h,
        &envelope(
            "reviewed",
            json!([
                { "surface": "github", "repo": "acme/widgets", "pr": 7 },
                { "surface": "discord", "channel": "whatever" }
            ]),
        ),
        1,
    )
    .await;
    let (code, body) = list(&h).await;
    assert_eq!(code, StatusCode::OK);
    let rows: Vec<ResultDelivery> = serde_json::from_value(body).unwrap();
    assert_eq!(rows.len(), 2);
    let github = rows.iter().find(|r| r.surface == "github").unwrap();
    let discord = rows.iter().find(|r| r.surface == "discord").unwrap();
    assert_eq!(github.status, status::SKIPPED);
    assert_eq!(discord.status, status::SKIPPED);

    let (code, body) = replay(&h, github.id).await;
    assert_eq!(code, StatusCode::OK);
    let replayed: ResultDelivery = serde_json::from_value(body).unwrap();
    assert_eq!(
        replayed.status,
        status::SKIPPED,
        "delivery status is not allowlist policy — still unblessed stays skipped"
    );
    assert!(
        h.store
            .claim_next_due_egress(Utc::now(), 120)
            .await
            .unwrap()
            .is_none(),
        "must not enqueue past the allowlist"
    );

    let (code, _) = replay(&h, discord.id).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);

    let (code, _) = replay(&h, ResultDeliveryId::new()).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}
