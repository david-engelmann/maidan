//! `secret://` refs on every egress path, end to end: the automation HTTP
//! deliveries of a slash command (the synchronous POST and the queued retry the
//! worker sends) and an A2A push notification carry the workspace's value to a
//! host the workspace trusts, the literal ref to any other, and the value is
//! written nowhere: not in the queued row, the event log, the audit trail or
//! any other table.

// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::sync::{
    atomic::{AtomicBool, AtomicI64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use axum::{body::Bytes, extract::State, http::StatusCode, routing::post, Router};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime, SlashRuntime, WebhookRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};

/// A value that needs JSON escaping, around a marker that does not, so a scan
/// finds it whether it was stored raw or escaped.
const MARKER: &str = "live-7f3a91";
const VALUE: &str = "sk-live-7f3a91\"q\nz";

fn key() -> Arc<[u8; 32]> {
    Arc::new([0x5e; 32])
}

#[derive(Clone, Default)]
struct Receiver {
    fail: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<(axum::http::HeaderMap, String)>>>,
}

impl Receiver {
    fn bodies(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, b)| b.clone())
            .collect()
    }

    fn last(&self) -> (axum::http::HeaderMap, String) {
        self.seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a delivery")
    }

    async fn wait_for(&self, n: usize) {
        for _ in 0..100 {
            if self.seen.lock().unwrap().len() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "receiver saw {} of {n} deliveries",
            self.seen.lock().unwrap().len()
        );
    }
}

async fn receive(
    State(r): State<Receiver>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> (StatusCode, axum::Json<Value>) {
    r.seen
        .lock()
        .unwrap()
        .push((headers, String::from_utf8_lossy(&body).into_owned()));
    if r.fail.load(Ordering::SeqCst) {
        (StatusCode::SERVICE_UNAVAILABLE, axum::Json(json!({})))
    } else {
        (StatusCode::OK, axum::Json(json!({})))
    }
}

struct H {
    base: String,
    http: reqwest::Client,
    state: AppState,
    store: Arc<dyn Store>,
    pool: SqlitePool,
    wid: String,
    tid: String,
    token: String,
    receiver: Receiver,
    receiver_url: String,
    _dir: tempfile::TempDir,
}

async fn spawn() -> H {
    std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1");
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(256)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, Some(key())),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.webhooks = WebhookRuntime::new(Some(key()));
    state.slash = SlashRuntime::new(Some(key()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let receiver = Receiver::default();
    let receiver_app = Router::new()
        .route("/r", post(receive))
        .with_state(receiver.clone());
    let receiver_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let receiver_addr = receiver_listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(receiver_listener, receiver_app).await.unwrap() });

    let ws = store
        .create_workspace(NewWorkspace {
            name: "egress".into(),
        })
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
            title: Some("t".into()),
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: [
                capability::WORKSPACE_READ,
                capability::WORKSPACE_WRITE,
                capability::MESSAGE_POST,
                capability::SECRET_ADMIN,
                capability::SECRET_READ,
            ]
            .iter()
            .map(|c| c.to_string())
            .collect(),
            expires_at: None,
        })
        .await
        .unwrap();

    let h = H {
        base: format!("http://{addr}"),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        state,
        store,
        pool,
        wid: ws.id.0.to_string(),
        tid: thread.id.0.to_string(),
        token: secret.as_str().to_string(),
        receiver,
        receiver_url: format!("http://{receiver_addr}/r"),
        _dir: dir,
    };
    let (status, _) = h
        .call(
            reqwest::Method::POST,
            &format!("/workspaces/{}/secrets", h.wid),
            Some(json!({ "name": "api-key", "value": VALUE })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    h
}

impl H {
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn trust_receiver(&self) {
        let (status, body) = self
            .call(
                reqwest::Method::POST,
                &format!("/workspaces/{}/secret-egress-hosts", self.wid),
                Some(json!({ "host": "127.0.0.1" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    async fn distrust_receiver(&self) {
        let (status, _) = self
            .call(
                reqwest::Method::DELETE,
                &format!("/workspaces/{}/secret-egress-hosts/127.0.0.1", self.wid),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    /// Every text value in every table: the value must be in none of them.
    async fn database_mentions(&self, needle: &str) -> Vec<String> {
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table'")
                .fetch_all(&self.pool)
                .await
                .unwrap();
        let mut hits = Vec::new();
        for table in tables {
            let rows = sqlx::query(&format!("SELECT * FROM \"{table}\""))
                .fetch_all(&self.pool)
                .await
                .unwrap();
            for row in rows {
                for i in 0..row.len() {
                    if let Ok(Some(text)) = row.try_get::<Option<String>, _>(i) {
                        if text.contains(needle) {
                            hits.push(format!("{table}: {text}"));
                        }
                    }
                }
            }
        }
        hits
    }
}

fn text_of(body: &str) -> String {
    let payload: Value = serde_json::from_str(body).expect("the delivered body is JSON");
    payload["text"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn automation_http_carries_the_value_only_to_a_trusted_host() {
    let h = spawn().await;
    let (status, command) = h
        .call(
            reqwest::Method::POST,
            &format!("/workspaces/{}/slash-commands", h.wid),
            Some(json!({
                "name": "deploy",
                "handler_kind": "http",
                "handler_target": h.receiver_url,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{command}");
    let signing = command["secret"].as_str().unwrap().to_string();
    let post = |n: usize| {
        let h = &h;
        async move {
            let (status, _) = h
                .call(
                    reqwest::Method::POST,
                    &format!("/threads/{}/messages", h.tid),
                    Some(json!({ "body": "/deploy secret://api-key" })),
                )
                .await;
            assert_eq!(status, StatusCode::CREATED);
            h.receiver.wait_for(n).await;
        }
    };

    // Not trusted: the synchronous POST carries the literal ref.
    post(1).await;
    assert_eq!(text_of(&h.receiver.last().1), "secret://api-key");

    // Trusted: the value, JSON-escaped, and the signature covers what was sent.
    h.trust_receiver().await;
    post(2).await;
    let (headers, body) = h.receiver.last();
    assert_eq!(text_of(&body), VALUE);
    let signature = headers["x-maidan-signature"].to_str().unwrap();
    assert!(maidan_server::webhooks::verify_signature(
        &signing, &body, signature
    ));

    // A failed POST is queued with the literal ref; the worker substitutes
    // again when it sends the retry.
    h.receiver.fail.store(true, Ordering::SeqCst);
    post(3).await;
    let queued: Vec<String> =
        sqlx::query_scalar("SELECT payload FROM maidan_automation_deliveries")
            .fetch_all(&h.pool)
            .await
            .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        text_of(&queued[0]),
        "secret://api-key",
        "the queued row holds the ref"
    );
    h.receiver.fail.store(false, Ordering::SeqCst);
    maidan_server::automation_worker::poll_once(&h.state, 16)
        .await
        .unwrap();
    h.receiver.wait_for(4).await;
    assert_eq!(text_of(&h.receiver.last().1), VALUE);

    // Revoked: the next queued retry carries the literal ref again.
    h.distrust_receiver().await;
    h.receiver.fail.store(true, Ordering::SeqCst);
    post(5).await;
    h.receiver.fail.store(false, Ordering::SeqCst);
    maidan_server::automation_worker::poll_once(&h.state, 16)
        .await
        .unwrap();
    h.receiver.wait_for(6).await;
    assert_eq!(text_of(&h.receiver.last().1), "secret://api-key");

    let delivered = h
        .receiver
        .bodies()
        .iter()
        .filter(|b| b.contains(MARKER))
        .count();
    assert_eq!(
        delivered, 3,
        "the value went out exactly while the host was trusted"
    );
    let hits = h.database_mentions(MARKER).await;
    assert!(hits.is_empty(), "the value was written down: {hits:?}");
    // Recorded in the workspace whose allowlist changed.
    let ws = maidan_types::WorkspaceId(uuid::Uuid::parse_str(&h.wid).unwrap());
    let audit = h.store.list_audit_for_workspace(ws, 100).await.unwrap();
    for action in ["secret_egress_host.allow", "secret_egress_host.revoke"] {
        assert!(
            audit
                .iter()
                .any(|row| row.action == action && row.metadata["host"] == "127.0.0.1"),
            "{action} unrecorded"
        );
    }
}

#[tokio::test]
async fn an_a2a_push_carries_the_value_only_to_a_trusted_host() {
    let h = spawn().await;
    let send = |n: usize| {
        let h = &h;
        async move {
            let resp: Value = h
                .http
                .post(format!("{}/a2a/v1/rpc", h.base))
                .bearer_auth(&h.token)
                .header("A2A-Version", "1.0")
                .json(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "SendMessage",
                    "params": {
                        "message": {
                            "messageId": uuid::Uuid::now_v7().to_string(),
                            "contextId": "deploy-secret://api-key",
                            "role": "ROLE_USER",
                            "parts": [{ "text": "ship it" }],
                        },
                        "configuration": { "taskPushNotificationConfig": { "url": h.receiver_url } },
                    },
                }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert!(resp.get("error").is_none(), "SendMessage failed: {resp}");
            h.receiver.wait_for(n).await;
            let (headers, body) = h.receiver.last();
            assert_eq!(headers["content-type"], "application/json");
            let pushed: Value = serde_json::from_str(&body).expect("the push is JSON");
            pushed["task"]["contextId"].as_str().unwrap().to_string()
        }
    };

    assert_eq!(send(1).await, "deploy-secret://api-key", "untrusted host");
    h.trust_receiver().await;
    assert_eq!(send(2).await, format!("deploy-{VALUE}"), "trusted host");
    h.distrust_receiver().await;
    assert_eq!(send(3).await, "deploy-secret://api-key", "revoked host");

    let hits = h.database_mentions(MARKER).await;
    assert!(hits.is_empty(), "the value was written down: {hits:?}");
}

#[tokio::test]
async fn trusting_a_host_needs_secret_read_and_a_bare_hostname() {
    let h = spawn().await;
    let ws = maidan_types::WorkspaceId(uuid::Uuid::parse_str(&h.wid).unwrap());
    let member = h
        .store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "rotator".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let rotator = TokenSecret::generate();
    h.store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(rotator.as_str()),
            label: None,
            capabilities: vec![capability::SECRET_ADMIN.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let path = format!("{}/workspaces/{}/secret-egress-hosts", h.base, h.wid);
    let resp = h
        .http
        .post(&path)
        .bearer_auth(rotator.as_str())
        .json(&json!({ "host": "127.0.0.1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(resp.text().await.unwrap().contains("secret:read"));
    let listed: Value = h
        .http
        .get(&path)
        .bearer_auth(rotator.as_str())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed, json!([]), "a secret:admin token may read the list");

    for bad in ["https://127.0.0.1", "127.0.0.1:8080", "*.example.com"] {
        let (status, _) = h
            .call(
                reqwest::Method::POST,
                &format!("/workspaces/{}/secret-egress-hosts", h.wid),
                Some(json!({ "host": bad })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (status, _) = h
        .call(
            reqwest::Method::DELETE,
            &format!(
                "/workspaces/{}/secret-egress-hosts/unlisted.example.com",
                h.wid
            ),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
