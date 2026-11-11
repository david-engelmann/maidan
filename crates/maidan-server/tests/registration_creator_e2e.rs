//! SEC-017: outbound webhooks, the mention webhook and slash commands act for
//! the token that registered them, as FSM hooks do (`fsm_hook_creator_e2e`).
//!
//! Each surface fires while its creator is live, and stops once SCIM
//! deactivates the creator's member, which revokes every token it holds. A
//! webhook also delivers only what its creator may read.

// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{expand_set, hash_secret, TokenSecret, AGENT_WORKER, HUMAN_ADMIN};
use maidan_bus::InMemoryBus;
use maidan_server::{
    router, webhook_worker::WebhookWorker, AppState, SlashRuntime, WebhookRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApiTokenId, AuditScope, ChannelId, ChannelMemberRole, MemberId, MemberKind, NewApiToken,
    NewAuditEvent, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

type Hits = Arc<Mutex<Vec<(String, Value)>>>;

struct Harness {
    base: String,
    receiver: String,
    hits: Hits,
    store: Arc<dyn Store>,
    client: reqwest::Client,
    worker: Option<WebhookWorker>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Harness {
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
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
    let key = Some(Arc::new([0x42u8; 32]));
    state.webhooks = WebhookRuntime::new(key.clone());
    state.slash = SlashRuntime::new(key);
    let worker = WebhookWorker::spawn(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // One receiver for every surface: it records the path and the body.
    let hits: Hits = Arc::default();
    let sink = hits.clone();
    let receiver = axum::Router::new().route(
        "/{surface}",
        axum::routing::post(
            move |axum::extract::Path(surface): axum::extract::Path<String>,
                  axum::Json(body): axum::Json<Value>| {
                let sink = sink.clone();
                async move {
                    sink.lock().unwrap().push((surface, body));
                    axum::Json(json!({ "text": "ok" }))
                }
            },
        ),
    );
    let receiver_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let receiver_addr = receiver_listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(receiver_listener, receiver).await.unwrap() });

    Harness {
        base: format!("http://{addr}"),
        receiver: format!("http://{receiver_addr}"),
        hits,
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

/// A workspace with a writer who posts, a creator who registers, and a
/// public and a private channel with a thread each. Only the writer is in the
/// private channel.
struct Case {
    ws: WorkspaceId,
    writer: Minted,
    writer_id: MemberId,
    public: ThreadId,
    private: ThreadId,
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

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, set: &str) -> Minted {
    let secret = TokenSecret::generate();
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: expand_set(set).unwrap(),
            expires_at: None,
        })
        .await
        .unwrap();
    Minted {
        secret: secret.as_str().to_string(),
        id: token.id,
    }
}

/// Deactivate the member holding `token` as SCIM does: its tokens are revoked
/// in the same transaction.
async fn deactivate(store: &dyn Store, ws: WorkspaceId, token: ApiTokenId) {
    let member = store.get_api_token(token).await.unwrap().member_id;
    store
        .create_scim_user(member, ws, None, true)
        .await
        .unwrap();
    store
        .scim_update_user_audited(
            ws,
            member,
            None,
            None,
            false,
            NewAuditEvent {
                scope: AuditScope::Workspace(ws),
                actor_id: None,
                action: "scim.user.deactivate".into(),
                target_kind: Some("member".into()),
                target_id: Some(member.0),
                metadata: json!({ "active": false }),
            },
        )
        .await
        .unwrap()
        .expect("the SCIM user exists");
}

async fn thread_in(store: &dyn Store, channel: ChannelId) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id
}

impl Harness {
    async fn case(&self, name: &str) -> Case {
        let store = self.store.as_ref();
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap()
            .id;
        let writer_id = member(store, ws, "writer").await;
        let writer = mint(store, ws, writer_id, HUMAN_ADMIN).await;
        let mut threads = Vec::new();
        for private in [false, true] {
            let channel = store
                .create_channel(NewChannel {
                    workspace_id: ws,
                    name: if private { "secret" } else { "general" }.into(),
                    topic: None,
                    private,
                })
                .await
                .unwrap()
                .id;
            if private {
                store
                    .add_channel_member(channel, writer_id, ChannelMemberRole::Member)
                    .await
                    .unwrap();
            }
            threads.push(thread_in(store, channel).await);
        }
        Case {
            ws,
            writer,
            writer_id,
            public: threads[0],
            private: threads[1],
        }
    }

    async fn creator(&self, c: &Case, set: &str) -> Minted {
        let id = member(self.store.as_ref(), c.ws, "creator").await;
        mint(self.store.as_ref(), c.ws, id, set).await
    }

    async fn call(&self, bearer: &str, method: reqwest::Method, path: &str, body: Value) -> Value {
        let resp = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(bearer)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let body = resp.json().await.unwrap_or(Value::Null);
        assert!(
            status.is_success() || status == StatusCode::NO_CONTENT,
            "{path}: {status} {body}"
        );
        body
    }

    async fn post(&self, c: &Case, thread: ThreadId, text: &str) -> Value {
        self.call(
            &c.writer.secret,
            reqwest::Method::POST,
            &format!("/threads/{}/messages", thread.0),
            json!({ "body": text }),
        )
        .await
    }

    /// The bodies `surface` received that mention `marker` anywhere.
    fn received(&self, surface: &str, marker: &str) -> usize {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, body)| s == surface && body.to_string().contains(marker))
            .count()
    }

    async fn received_within(&self, surface: &str, marker: &str, within: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        while tokio::time::Instant::now() < deadline {
            if self.received(surface, marker) > 0 {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }

    async fn shutdown(mut self) {
        if let Some(worker) = self.worker.take() {
            worker.shutdown().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_webhook_delivers_only_what_its_creator_reads_and_stops_when_the_creator_is_deactivated()
{
    let h = spawn().await;
    let c = h.case("webhook").await;
    let admin = h.creator(&c, HUMAN_ADMIN).await;
    let hook = h
        .call(
            &admin.secret,
            reqwest::Method::POST,
            &format!("/workspaces/{}/webhooks", c.ws.0),
            json!({ "url": format!("{}/hook", h.receiver), "event_kinds": ["message_posted"] }),
        )
        .await;
    assert!(hook["webhook"]["id"].is_string(), "{hook}");

    h.post(&c, c.private, "private-before").await;
    h.post(&c, c.public, "public-before").await;
    assert!(
        h.received_within("hook", "public-before", Duration::from_secs(5))
            .await,
        "the live admin's webhook did not deliver"
    );
    assert_eq!(
        h.received("hook", "private-before"),
        0,
        "a channel its creator is not in reached the webhook"
    );

    deactivate(h.store.as_ref(), c.ws, admin.id).await;
    h.post(&c, c.public, "public-after").await;
    assert!(
        !h.received_within("hook", "public-after", Duration::from_secs(1))
            .await,
        "a deactivated creator's webhook still delivered"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_mention_webhook_stops_when_its_creator_is_deactivated() {
    let h = spawn().await;
    let c = h.case("mention").await;
    let admin = h.creator(&c, HUMAN_ADMIN).await;
    // Listens for nothing itself: only its choice as the mention webhook
    // sends it mentions.
    let hook = h
        .call(
            &admin.secret,
            reqwest::Method::POST,
            &format!("/workspaces/{}/webhooks", c.ws.0),
            json!({ "url": format!("{}/mention", h.receiver), "event_kinds": ["vote_cast"] }),
        )
        .await;
    h.call(
        &admin.secret,
        reqwest::Method::PUT,
        &format!("/workspaces/{}/mention-webhook", c.ws.0),
        json!({ "webhook_id": hook["webhook"]["id"] }),
    )
    .await;
    let mention = |message: Value| {
        let h = &h;
        let c = &c;
        async move {
            h.call(
                &c.writer.secret,
                reqwest::Method::POST,
                &format!("/messages/{}/mentions", message["id"].as_str().unwrap()),
                json!({ "member_id": c.writer_id.0 }),
            )
            .await;
            message["id"].as_str().unwrap().to_string()
        }
    };

    let before = mention(h.post(&c, c.public, "hi").await).await;
    assert!(
        h.received_within("mention", &before, Duration::from_secs(5))
            .await,
        "the live admin's mention webhook did not deliver"
    );

    deactivate(h.store.as_ref(), c.ws, admin.id).await;
    let after = mention(h.post(&c, c.public, "hi again").await).await;
    assert!(
        !h.received_within("mention", &after, Duration::from_secs(1))
            .await,
        "a deactivated creator's mention webhook still delivered"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slash_command_runs_only_while_its_creator_is_live_and_only_where_it_reads() {
    let h = spawn().await;
    let c = h.case("slash").await;
    let worker = h.creator(&c, AGENT_WORKER).await;
    h.call(
        &worker.secret,
        reqwest::Method::POST,
        &format!("/workspaces/{}/slash-commands", c.ws.0),
        json!({
            "name": "ping",
            "handler_kind": "http",
            "handler_target": format!("{}/slash", h.receiver),
        }),
    )
    .await;
    let outcome = |msg: Value| msg["metadata"]["slash_response"].clone();

    let fired = outcome(h.post(&c, c.public, "/ping live").await);
    assert_eq!(fired["ok"], true, "{fired}");
    assert_eq!(h.received("slash", "live"), 1);

    let hidden = outcome(h.post(&c, c.private, "/ping hidden").await);
    assert_eq!(hidden["error"], "command_inactive", "{hidden}");
    assert_eq!(h.received("slash", "hidden"), 0);

    deactivate(h.store.as_ref(), c.ws, worker.id).await;
    let gone = outcome(h.post(&c, c.public, "/ping gone").await);
    assert_eq!(gone["error"], "command_inactive", "{gone}");
    assert_eq!(gone["reason"], "creator token no longer valid");
    assert_eq!(h.received("slash", "gone"), 0);
    h.shutdown().await;
}
