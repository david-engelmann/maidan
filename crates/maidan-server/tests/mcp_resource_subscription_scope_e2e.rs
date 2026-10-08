//! MCP resource notifications reach only the caller that subscribed, in the
//! session it subscribed in, for resources it can still read in its own
//! workspace.
//!
//! Subscriptions were one process-wide set, then one set per workspace. The
//! second still told every listener in a workspace about private threads any
//! member watched, and a content-addressed artifact's URI names a resource in
//! every workspace that uploaded the same bytes, so one tenant's upload was
//! delivered to another tenant watching its own copy. Each scenario runs on
//! SQLite in-process and on Postgres through the cross-replica notifier.

mod common;

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use base64::Engine as _;
use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

async fn spawn(
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
    notifier: Option<Arc<dyn maidan_bus::ResourceNotifier>>,
) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    if let Some(notifier) = notifier {
        state.attach_resource_notifier(notifier);
        state.mcp.spawn_resource_notify_listener();
    }
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        server,
        _dir: dir,
    }
}

async fn sqlite_harness() -> Harness {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    spawn(
        store,
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        None,
    )
    .await
}

async fn postgres_harness() -> Option<(
    testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    Harness,
)> {
    let (container, pool) = common::postgres_pool().await?;
    let notifier = maidan_bus::PostgresResourceNotifier::connect(pool.clone())
        .await
        .expect("connect the resource notifier");
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let harness = spawn(
        store,
        Arc::new(maidan_search::PostgresSearch::new(pool)),
        Some(Arc::new(notifier)),
    )
    .await;
    // Let the LISTEN task attach before anything is published.
    tokio::time::sleep(Duration::from_millis(300)).await;
    Some((container, harness))
}

/// A member of `ws` and a bearer token for it.
async fn member_token(store: &dyn Store, ws: WorkspaceId, handle: &str) -> String {
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
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
                capability::ARTIFACT_UPLOAD.into(),
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

impl Harness {
    async fn rest(&self, token: &str, path: &str, body: Value) -> Value {
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "{path}: {}", resp.status());
        resp.json().await.unwrap()
    }

    /// A thread transition over REST, returning the status and the body.
    async fn transition(
        &self,
        token: &str,
        thread_id: &str,
        action: &str,
    ) -> (reqwest::StatusCode, String) {
        let resp = self
            .client
            .post(format!("{}/threads/{thread_id}", self.base))
            .bearer_auth(token)
            .json(&json!({ "action": action }))
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.text().await.unwrap())
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        self.rest(
            token,
            "/mcp",
            json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
        )
        .await
    }

    async fn thread(&self, token: &str, ws: WorkspaceId, private: bool) -> String {
        let channel = self
            .rest(
                token,
                &format!("/workspaces/{}/channels", ws.0),
                json!({ "name": format!("c-{}", uuid::Uuid::new_v4()), "private": private }),
            )
            .await;
        let channel_id = channel["id"].as_str().unwrap();
        let thread = self
            .rest(
                token,
                &format!("/channels/{channel_id}/threads"),
                json!({ "title": "t" }),
            )
            .await;
        thread["id"].as_str().unwrap().to_string()
    }

    async fn post(&self, token: &str, thread_id: &str) {
        let resp = self
            .rpc(
                token,
                "tools/call",
                json!({ "name": "post_message",
                        "arguments": { "thread_id": thread_id, "body": "update" } }),
            )
            .await;
        assert!(resp["error"].is_null(), "post_message: {resp}");
    }

    async fn upload(&self, token: &str, bytes: &[u8]) -> String {
        let resp = self
            .rpc(
                token,
                "tools/call",
                json!({ "name": "upload_artifact", "arguments": {
                    "kind": "attachment",
                    "content_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
                }}),
            )
            .await;
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        body["sha256"].as_str().unwrap().to_string()
    }

    async fn subscribe(&self, token: &str, uri: &str) -> Value {
        self.rpc(token, "resources/subscribe", json!({ "uri": uri }))
            .await
    }

    /// An SSE listener on `path`, collecting everything it receives.
    async fn listen(&self, token: &str, path: &str) -> Listener {
        let resp = self
            .client
            .get(format!("{}{path}", self.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "{path}: {}", resp.status());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut body = resp.bytes_stream();
            while let Some(Ok(chunk)) = body.next().await {
                if tx
                    .send(String::from_utf8_lossy(&chunk).to_string())
                    .is_err()
                {
                    break;
                }
            }
        });
        Listener { rx, task }
    }
}

struct Listener {
    rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    task: tokio::task::JoinHandle<()>,
}

impl Listener {
    /// Everything received until `window` passes with `until` unseen.
    async fn collect(&mut self, until: &str, window: Duration) -> String {
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + window;
        while !seen.contains(until) {
            match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Ok(Some(chunk)) => seen.push_str(&chunk),
                _ => break,
            }
        }
        seen
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

const UPDATED: &str = "notifications/resources/updated";

async fn a_tenant_hears_nothing_of_another_tenants_resources(h: &Harness) {
    let ws_a = workspace(h.store.as_ref(), "alpha").await;
    let ws_b = workspace(h.store.as_ref(), "bravo").await;
    let token_a = member_token(h.store.as_ref(), ws_a, "alpha-agent").await;
    let token_b = member_token(h.store.as_ref(), ws_b, "bravo-agent").await;
    let thread_a = h.thread(&token_a, ws_a, false).await;
    let thread_uri = format!("maidan://threads/{thread_a}");

    // B holds its own copy of some bytes and watches it.
    let shared = b"the same bytes in two workspaces";
    let sha = h.upload(&token_b, shared).await;
    let artifact_uri = format!("maidan://artifacts/{sha}");

    let mut listen_a = h.listen(&token_a, "/mcp/notifications").await;
    let mut listen_b = h.listen(&token_b, "/mcp/notifications").await;
    let mut listen_b_get = h.listen(&token_b, "/mcp/streamable").await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let refused = h.subscribe(&token_b, &thread_uri).await;
    assert!(
        refused["error"].is_object(),
        "B subscribed to A's thread: {refused}"
    );
    let foreign_ws = format!("maidan://workspaces/{}", ws_a.0);
    let refused = h.subscribe(&token_b, &foreign_ws).await;
    assert!(
        refused["error"].is_object(),
        "B subscribed to A's workspace: {refused}"
    );
    assert!(h.subscribe(&token_b, &artifact_uri).await["error"].is_null());
    assert!(h.subscribe(&token_a, &thread_uri).await["error"].is_null());

    // A uploads the same bytes and posts in its own thread.
    assert_eq!(h.upload(&token_a, shared).await, sha);
    h.post(&token_a, &thread_a).await;

    let got_a = listen_a.collect(&thread_uri, Duration::from_secs(5)).await;
    assert!(
        got_a.contains(&thread_uri),
        "A missed its own update: {got_a}"
    );
    for (name, listener) in [
        ("notifications", &mut listen_b),
        ("streamable", &mut listen_b_get),
    ] {
        let got = listener.collect(UPDATED, Duration::from_millis(800)).await;
        assert!(
            !got.contains(UPDATED),
            "B's {name} listener received tenant A's update: {got}"
        );
    }
}

async fn a_private_thread_is_heard_only_by_members_of_its_channel(h: &Harness) {
    let ws = workspace(h.store.as_ref(), "one-tenant").await;
    let alice = member_token(h.store.as_ref(), ws, "alice").await;
    let bob = member_token(h.store.as_ref(), ws, "bob").await;
    // Alice creates the private channel, so only she is a member.
    let secret = h.thread(&alice, ws, true).await;
    let secret_uri = format!("maidan://threads/{secret}");
    let public = h.thread(&bob, ws, false).await;
    let public_uri = format!("maidan://threads/{public}");

    let mut listen_alice = h.listen(&alice, "/mcp/notifications").await;
    let mut listen_bob = h.listen(&bob, "/mcp/notifications").await;
    let mut listen_bob_get = h.listen(&bob, "/mcp/streamable").await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let refused = h.subscribe(&bob, &secret_uri).await;
    assert!(
        refused["error"].is_object(),
        "a non-member subscribed to a private thread: {refused}"
    );
    assert!(h.subscribe(&alice, &secret_uri).await["error"].is_null());
    assert!(h.subscribe(&bob, &public_uri).await["error"].is_null());

    h.post(&alice, &secret).await;
    h.post(&bob, &public).await;

    let got_alice = listen_alice
        .collect(&secret_uri, Duration::from_secs(5))
        .await;
    assert!(got_alice.contains(&secret_uri), "{got_alice}");
    assert!(
        !got_alice.contains(&public_uri),
        "Alice heard Bob's subscription: {got_alice}"
    );
    let got_bob = listen_bob
        .collect(&public_uri, Duration::from_secs(5))
        .await;
    assert!(got_bob.contains(&public_uri), "{got_bob}");
    let got_bob_get = listen_bob_get
        .collect(&public_uri, Duration::from_secs(5))
        .await;
    for got in [got_bob, got_bob_get] {
        assert!(
            !got.contains(&secret_uri),
            "a member outside the private channel heard about its thread: {got}"
        );
    }
}

/// A refused close over REST is a new notice message on the thread, so it
/// notifies that thread's subscribers the way the MCP twin does, and nobody
/// in another workspace.
async fn a_rest_close_refusal_is_heard_by_its_own_workspace_only(h: &Harness) {
    let ws_a = workspace(h.store.as_ref(), "close-alpha").await;
    let ws_b = workspace(h.store.as_ref(), "close-bravo").await;
    let token_a = member_token(h.store.as_ref(), ws_a, "close-alpha-agent").await;
    let token_b = member_token(h.store.as_ref(), ws_b, "close-bravo-agent").await;
    let thread_a = h.thread(&token_a, ws_a, false).await;
    let thread_b = h.thread(&token_b, ws_b, false).await;
    let thread_uri = format!("maidan://threads/{thread_a}");

    // One approval is required and none is given, so close is refused.
    let tid = maidan_types::ThreadId(uuid::Uuid::parse_str(&thread_a).unwrap());
    let owner = h.store.get_thread(tid).await.unwrap().owner_id.unwrap();
    h.store.set_review_requirement(tid, 1).await.unwrap();
    h.store
        .set_thread_result(tid, owner, &json!({ "done": true }))
        .await
        .unwrap();

    let mut listen_a = h.listen(&token_a, "/mcp/notifications").await;
    let mut listen_b = h.listen(&token_b, "/mcp/notifications").await;
    let mut listen_b_get = h.listen(&token_b, "/mcp/streamable").await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(h.subscribe(&token_a, &thread_uri).await["error"].is_null());
    for uri in [
        format!("maidan://workspaces/{}", ws_b.0),
        format!("maidan://threads/{thread_b}"),
    ] {
        assert!(h.subscribe(&token_b, &uri).await["error"].is_null());
    }

    // Going to review notifies A once; wait for it so the next update can only
    // come from the refusal.
    let (status, body) = h.transition(&token_a, &thread_a, "start_review").await;
    assert!(status.is_success(), "start_review: {status} {body}");
    let got = listen_a.collect(&thread_uri, Duration::from_secs(5)).await;
    assert!(got.contains(&thread_uri), "A missed start_review: {got}");

    let (status, body) = h.transition(&token_a, &thread_a, "close").await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT, "{body}");
    assert!(body.contains("review requirement not met"), "{body}");
    let notices = h
        .store
        .list_messages(tid, 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.metadata["notice"] == "transition_refused")
        .count();
    assert_eq!(notices, 1, "the refusal was not recorded on the thread");

    let got_a = listen_a.collect(&thread_uri, Duration::from_secs(5)).await;
    assert!(
        got_a.contains(&thread_uri),
        "A's subscriber missed the close refusal: {got_a}"
    );
    for (name, listener) in [
        ("notifications", &mut listen_b),
        ("streamable", &mut listen_b_get),
    ] {
        let got = listener.collect(UPDATED, Duration::from_millis(800)).await;
        assert!(
            !got.contains(UPDATED),
            "B's {name} listener heard tenant A's close refusal: {got}"
        );
    }
}

#[tokio::test]
async fn a_rest_close_refusal_is_heard_by_its_own_workspace_only_on_sqlite() {
    let h = sqlite_harness().await;
    a_rest_close_refusal_is_heard_by_its_own_workspace_only(&h).await;
    h.server.abort();
}

#[tokio::test]
async fn a_rest_close_refusal_is_heard_by_its_own_workspace_only_on_postgres() {
    let Some((_container, h)) = postgres_harness().await else {
        return;
    };
    a_rest_close_refusal_is_heard_by_its_own_workspace_only(&h).await;
    h.server.abort();
}

#[tokio::test]
async fn a_tenant_hears_nothing_of_another_tenants_resources_on_sqlite() {
    let h = sqlite_harness().await;
    a_tenant_hears_nothing_of_another_tenants_resources(&h).await;
    h.server.abort();
}

#[tokio::test]
async fn a_tenant_hears_nothing_of_another_tenants_resources_on_postgres() {
    let Some((_container, h)) = postgres_harness().await else {
        return;
    };
    a_tenant_hears_nothing_of_another_tenants_resources(&h).await;
    h.server.abort();
}

#[tokio::test]
async fn a_private_thread_is_heard_only_by_members_of_its_channel_on_sqlite() {
    let h = sqlite_harness().await;
    a_private_thread_is_heard_only_by_members_of_its_channel(&h).await;
    h.server.abort();
}

#[tokio::test]
async fn a_private_thread_is_heard_only_by_members_of_its_channel_on_postgres() {
    let Some((_container, h)) = postgres_harness().await else {
        return;
    };
    a_private_thread_is_heard_only_by_members_of_its_channel(&h).await;
    h.server.abort();
}
