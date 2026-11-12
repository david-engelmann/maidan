//! SEC-013 and SEC-009: registering automation that acts on, or receives, the
//! whole workspace is an administrator's call.
//!
//! - An FSM hook runs on every matching transition of any thread in the
//!   workspace, so registering one needs `workspace:admin`, over REST and MCP.
//! - An outbound webhook receives every matching event, private channels and
//!   DMs included, so creating one, or choosing the mention webhook, needs
//!   `workspace:admin`.
//!
//! A worker token (`maidan.agent.worker`) holds `workspace:write` and is
//! refused; an admin of the workspace is not; an admin of another workspace
//! is refused.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{expand_set, hash_secret, TokenSecret, AGENT_WORKER, HUMAN_ADMIN};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, WebhookRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
use serde_json::{json, Value};

struct Harness {
    base: String,
    store: Arc<dyn Store>,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Harness {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
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
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
    );
    state.webhooks = WebhookRuntime::new(Some(Arc::new([0x55; 32])));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        store,
        client: reqwest::Client::new(),
        _dir: dir,
    }
}

/// A workspace with an admin token and a worker token.
struct Tenant {
    workspace: WorkspaceId,
    admin: String,
    worker: String,
}

async fn token(store: &dyn Store, ws: WorkspaceId, handle: &str, set: &str) -> String {
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: expand_set(set).unwrap(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    Tenant {
        workspace: ws.id,
        admin: token(store, ws.id, &format!("{name}-admin"), HUMAN_ADMIN).await,
        worker: token(store, ws.id, &format!("{name}-worker"), AGENT_WORKER).await,
    }
}

impl Harness {
    async fn send(&self, request: reqwest::RequestBuilder) -> (StatusCode, Value) {
        let resp = request.send().await.unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn register_hook_rest(&self, ws: WorkspaceId, bearer: &str) -> (StatusCode, Value) {
        self.send(
            self.client
                .post(format!("{}/workspaces/{}/fsm-hooks", self.base, ws.0))
                .bearer_auth(bearer)
                .json(&hook_body()),
        )
        .await
    }

    async fn register_hook_mcp(&self, ws: WorkspaceId, bearer: &str) -> Value {
        let mut arguments = hook_body();
        arguments["workspace_id"] = json!(ws.0);
        self.send(
            self.client
                .post(format!("{}/mcp", self.base))
                .bearer_auth(bearer)
                .header("accept", "application/json, text/event-stream")
                .json(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {"name": "register_fsm_hook", "arguments": arguments},
                })),
        )
        .await
        .1
    }

    async fn register_slash_rest(
        &self,
        ws: WorkspaceId,
        bearer: &str,
        name: &str,
        tool: &str,
    ) -> (StatusCode, Value) {
        self.send(
            self.client
                .post(format!("{}/workspaces/{}/slash-commands", self.base, ws.0))
                .bearer_auth(bearer)
                .json(&slash_body(name, tool)),
        )
        .await
    }

    async fn register_slash_mcp(
        &self,
        ws: WorkspaceId,
        bearer: &str,
        name: &str,
        tool: &str,
    ) -> Value {
        let mut arguments = slash_body(name, tool);
        arguments["workspace_id"] = json!(ws.0);
        self.send(
            self.client
                .post(format!("{}/mcp", self.base))
                .bearer_auth(bearer)
                .header("accept", "application/json, text/event-stream")
                .json(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {"name": "register_slash_command", "arguments": arguments},
                })),
        )
        .await
        .1
    }

    async fn create_webhook(&self, ws: WorkspaceId, bearer: &str) -> (StatusCode, Value) {
        self.send(
            self.client
                .post(format!("{}/workspaces/{}/webhooks", self.base, ws.0))
                .bearer_auth(bearer)
                .json(&json!({
                    "url": "https://example.com/hook",
                    "label": "everything",
                    "event_kinds": ["message_posted"],
                })),
        )
        .await
    }

    async fn set_mention_webhook(
        &self,
        ws: WorkspaceId,
        bearer: &str,
        webhook_id: Value,
    ) -> StatusCode {
        self.send(
            self.client
                .put(format!("{}/workspaces/{}/mention-webhook", self.base, ws.0))
                .bearer_auth(bearer)
                .json(&json!({"webhook_id": webhook_id})),
        )
        .await
        .0
    }
}

/// The SEC-013 escalation: a hook that strips a thread's review requirement
/// whenever work goes into review.
fn hook_body() -> Value {
    json!({
        "label": "strip review",
        "to_state": "in_review",
        "handler_kind": "mcp_tool",
        "handler_target": "clear_review_requirement",
    })
}

/// RED-004: a slash command runs its tool as whoever types it.
fn slash_body(name: &str, tool: &str) -> Value {
    json!({
        "name": name,
        "handler_kind": "mcp_tool",
        "handler_target": tool,
    })
}

fn missing_capability(body: &Value, capability: &str) -> bool {
    body["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains(capability))
}

fn missing_admin(body: &Value) -> bool {
    body["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("workspace:admin"))
}

#[tokio::test]
async fn a_worker_token_cannot_register_an_fsm_hook_over_rest_or_mcp_and_an_admin_can() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;

    let (status, body) = h.register_hook_rest(a.workspace, &a.worker).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "worker over REST: {body}");
    let body = h.register_hook_mcp(a.workspace, &a.worker).await;
    assert!(missing_admin(&body), "worker over MCP: {body}");
    // Another workspace's admin holds workspace:admin, but not here.
    let (status, body) = h.register_hook_rest(a.workspace, &b.admin).await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "another workspace's admin over REST: {status} {body}"
    );
    let body = h.register_hook_mcp(a.workspace, &b.admin).await;
    assert!(
        body.get("error").is_some(),
        "another workspace's admin over MCP: {body}"
    );
    assert!(
        h.store
            .list_fsm_hooks(a.workspace)
            .await
            .unwrap()
            .is_empty(),
        "a refused registration left a hook behind"
    );

    let (status, body) = h.register_hook_rest(a.workspace, &a.admin).await;
    assert_eq!(status, StatusCode::CREATED, "admin over REST: {body}");
    let body = h.register_hook_mcp(a.workspace, &a.admin).await;
    assert!(body.get("result").is_some(), "admin over MCP: {body}");
    assert_eq!(h.store.list_fsm_hooks(a.workspace).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_worker_token_cannot_create_a_webhook_or_choose_the_mention_webhook_and_an_admin_can() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;

    let (status, body) = h.create_webhook(a.workspace, &a.worker).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "worker create: {body}");
    let (status, body) = h.create_webhook(a.workspace, &b.admin).await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "another workspace's admin create: {status} {body}"
    );
    assert!(
        h.store
            .list_webhook_subscriptions(a.workspace)
            .await
            .unwrap()
            .is_empty(),
        "a refused create left a subscription behind"
    );

    let (status, created) = h.create_webhook(a.workspace, &a.admin).await;
    assert_eq!(status, StatusCode::CREATED, "admin create: {created}");
    let webhook_id = created["webhook"]["id"].clone();
    assert!(webhook_id.is_string(), "{created}");

    for webhook in [webhook_id.clone(), Value::Null] {
        let status = h.set_mention_webhook(a.workspace, &a.worker, webhook).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "worker mention-webhook");
    }
    let status = h
        .set_mention_webhook(a.workspace, &b.admin, webhook_id.clone())
        .await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "another workspace's admin mention-webhook: {status}"
    );
    let status = h
        .set_mention_webhook(a.workspace, &a.admin, webhook_id)
        .await;
    assert_eq!(status, StatusCode::OK, "admin mention-webhook");
}

/// RED-004: a writer could bind `/help` to a destructive tool and wait for an
/// admin to type it, since the command runs as its invoker. Binding a command
/// to a tool that is not read-only takes `channel:admin`; a read-only tool
/// still takes only `workspace:write`.
#[tokio::test]
async fn a_worker_token_binds_a_slash_command_only_to_a_read_only_tool_and_an_admin_to_any() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;
    let destructive = "clear_review_requirement";

    let (status, body) = h
        .register_slash_rest(a.workspace, &a.worker, "help", destructive)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "worker over REST: {body}");
    let body = h
        .register_slash_mcp(a.workspace, &a.worker, "help", destructive)
        .await;
    assert!(
        missing_capability(&body, "channel:admin"),
        "worker over MCP: {body}"
    );
    let (status, body) = h
        .register_slash_rest(a.workspace, &b.admin, "help", destructive)
        .await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "another workspace's admin over REST: {status} {body}"
    );
    let body = h
        .register_slash_mcp(a.workspace, &b.admin, "help", destructive)
        .await;
    assert!(
        body.get("error").is_some(),
        "another workspace's admin over MCP: {body}"
    );
    assert!(
        h.store
            .list_slash_commands(a.workspace)
            .await
            .unwrap()
            .is_empty(),
        "a refused registration left a command behind"
    );

    // A read-only tool is still a writer's to bind.
    let (status, body) = h
        .register_slash_rest(a.workspace, &a.worker, "channels", "list_channels")
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "worker, read-only tool: {body}"
    );
    let body = h
        .register_slash_mcp(a.workspace, &a.worker, "channels-mcp", "list_channels")
        .await;
    assert!(
        body.get("result").is_some(),
        "worker over MCP, read-only tool: {body}"
    );

    let (status, body) = h
        .register_slash_rest(a.workspace, &a.admin, "help", destructive)
        .await;
    assert_eq!(status, StatusCode::CREATED, "admin over REST: {body}");
    let body = h
        .register_slash_mcp(a.workspace, &a.admin, "help-mcp", destructive)
        .await;
    assert!(body.get("result").is_some(), "admin over MCP: {body}");
    assert_eq!(
        h.store
            .list_slash_commands(a.workspace)
            .await
            .unwrap()
            .len(),
        4
    );
    assert!(h
        .store
        .list_slash_commands(b.workspace)
        .await
        .unwrap()
        .is_empty());
}
