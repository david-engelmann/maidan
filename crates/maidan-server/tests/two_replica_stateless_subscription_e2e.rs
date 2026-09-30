//! A stateless MCP subscription is delivered by whichever replica holds the
//! caller's listener.
//!
//! Two servers over one Postgres database stand in for two replicas behind a
//! load balancer with no affinity: a stateless client's `resources/subscribe`
//! lands on one and its `GET /mcp/notifications` on the other. The
//! subscription used to be held by the replica that took it, so the listener
//! heard nothing. Two tenants watch the same content-addressed artifact from
//! opposite replicas, so a cross-replica lookup that forgot the workspace
//! would show up as a leak.

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
use maidan_store::prelude::*;
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use serde_json::{json, Value};
use sqlx::PgPool;

struct Replica {
    base: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Replica {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// A replica with its own pool, store and NOTIFY listener, over the shared
/// database and object store.
async fn replica(pool: &PgPool, artifacts: &std::path::Path) -> Replica {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(6)
        .acquire_timeout(Duration::from_secs(15))
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("replica pool");
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let mut state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(artifacts)),
        Arc::new(maidan_bus::InMemoryBus::new()),
        Arc::new(maidan_search::PostgresSearch::new(pool.clone())),
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let notifier = maidan_bus::PostgresResourceNotifier::connect(pool)
        .await
        .expect("connect the resource notifier");
    state.attach_resource_notifier(Arc::new(notifier));
    state.mcp.spawn_resource_notify_listener();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Replica {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        server,
    }
}

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
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

impl Replica {
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

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        self.rest(
            token,
            "/mcp",
            json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
        )
        .await
    }

    async fn thread(&self, token: &str, ws: WorkspaceId) -> String {
        let channel = self
            .rest(
                token,
                &format!("/workspaces/{}/channels", ws.0),
                json!({ "name": format!("c-{}", uuid::Uuid::new_v4()) }),
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

    async fn subscribe(&self, token: &str, uri: &str) {
        let resp = self
            .rpc(token, "resources/subscribe", json!({ "uri": uri }))
            .await;
        assert!(resp["error"].is_null(), "subscribe {uri}: {resp}");
    }

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
        self.collect_all(&[until], window).await
    }

    /// Everything received until `window` passes with any of `until` unseen.
    async fn collect_all(&mut self, until: &[&str], window: Duration) -> String {
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + window;
        while !until.iter().all(|u| seen.contains(u)) {
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

#[tokio::test]
async fn a_stateless_subscription_is_delivered_by_the_replica_holding_the_listener() {
    let Some((_container, pool)) = common::postgres_pool().await else {
        return;
    };
    let objects = tempfile::tempdir().unwrap();
    let one = replica(&pool, objects.path()).await;
    let two = replica(&pool, objects.path()).await;
    // Let both LISTEN tasks attach before anything is published.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let store = PostgresStore::for_tests(pool.clone());
    let ws_a = store
        .create_workspace(NewWorkspace {
            name: "alpha".into(),
        })
        .await
        .unwrap()
        .id;
    let ws_b = store
        .create_workspace(NewWorkspace {
            name: "bravo".into(),
        })
        .await
        .unwrap()
        .id;
    let alpha = member_token(&store, ws_a, "alpha-agent").await;
    let bravo = member_token(&store, ws_b, "bravo-agent").await;
    let thread = one.thread(&alpha, ws_a).await;
    let thread_uri = format!("maidan://threads/{thread}");
    let shared = b"the same bytes in two workspaces, on two replicas";
    let sha = two.upload(&bravo, shared).await;
    assert_eq!(one.upload(&alpha, shared).await, sha);
    let artifact_uri = format!("maidan://artifacts/{sha}");

    // Each tenant subscribes on one replica and listens on the other.
    let mut alpha_on_two = two.listen(&alpha, "/mcp/notifications").await;
    let mut alpha_on_two_get = two.listen(&alpha, "/mcp/streamable").await;
    let mut bravo_on_one = one.listen(&bravo, "/mcp/notifications").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    one.subscribe(&alpha, &thread_uri).await;
    one.subscribe(&alpha, &artifact_uri).await;
    two.subscribe(&bravo, &artifact_uri).await;

    // Alpha uploads its bytes again and posts, both on replica one.
    one.upload(&alpha, shared).await;
    one.post(&alpha, &thread).await;

    for (name, listener) in [
        ("notifications", &mut alpha_on_two),
        ("streamable", &mut alpha_on_two_get),
    ] {
        let got = listener
            .collect_all(&[&thread_uri, &artifact_uri], Duration::from_secs(5))
            .await;
        assert!(
            got.contains(&thread_uri) && got.contains(&artifact_uri),
            "alpha's {name} listener on replica two missed its updates: {got}"
        );
    }
    let leaked = bravo_on_one
        .collect(UPDATED, Duration::from_millis(800))
        .await;
    assert!(
        !leaked.contains(UPDATED),
        "bravo's listener on replica one heard alpha's update: {leaked}"
    );

    // Bravo's own upload reaches bravo across the replicas, and not alpha.
    two.upload(&bravo, shared).await;
    let got = bravo_on_one
        .collect(&artifact_uri, Duration::from_secs(5))
        .await;
    assert!(got.contains(&artifact_uri), "{got}");

    // Unsubscribing on replica two ends what replica one took.
    let resp = two
        .rpc(
            &alpha,
            "resources/unsubscribe",
            json!({ "uri": thread_uri }),
        )
        .await;
    assert_eq!(resp["result"]["removed"], true, "{resp}");
    one.post(&alpha, &thread).await;
    let got = alpha_on_two
        .collect(&thread_uri, Duration::from_millis(800))
        .await;
    assert!(
        !got.contains(&thread_uri),
        "an unsubscribed thread was still delivered: {got}"
    );
}
