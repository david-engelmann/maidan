//! SEC-017, SEC-018 and SEC-012: every registration ends with its owner.
//!
//! Each case registers something as a member, deactivates that member through
//! SCIM (which revokes its tokens), and then makes the registration fire:
//!
//! - a webhook subscription and the workspace's mention route stop delivering,
//!   the subscription is revoked, and `webhook.skipped` records why; a live
//!   admin's subscription in the same workspace still delivers, and only what
//!   that admin may read;
//! - a delivery queued while its creator could read the event is not sent
//!   once the creator has left the private channel it happened in;
//! - a slash command answers that its creator is not authorized and sends
//!   nothing to its handler;
//! - an A2A push config sends nothing and is deleted;
//! - a task schedule is paused and opens no thread, and one whose creator has
//!   left its private channel is skipped and paused when it comes due;
//! - a share ticket no longer resolves and an invite no longer admits.
//!
//! Every case runs on SQLite and on Postgres. A last test pins the tenant
//! line: a registration never acts with another workspace's token, and ending
//! a member in one workspace leaves another workspace's registrations alone.

// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{body::Bytes, extract::State, http::StatusCode, http::Uri, Router};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{expand_set, hash_secret, TokenSecret, AGENT_WORKER, HUMAN_ADMIN};
use maidan_bus::InMemoryBus;
use maidan_server::{
    router, scheduler, webhook_worker, webhook_worker::WebhookWorker, AppState, FederationRuntime,
    SlashRuntime, WebhookRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations, A2aTaskWrite};
use maidan_types::{
    ApiTokenId, ChannelId, ChannelMemberRole, EventKind, MemberId, MemberKind, NewApiToken,
    NewAuditEvent, NewChannel, NewMember, NewShareTicket, NewTaskSchedule, NewThread,
    NewWebhookSubscription, NewWorkspace, NewWorkspaceInvite, ThreadId, WebhookSubscriptionId,
    WorkspaceId, WorkspaceInviteRole,
};
use serde_json::{json, Value};

const KEY: [u8; 32] = [0x2c; 32];

type Seen = Arc<Mutex<Vec<(String, Value)>>>;

struct Backend {
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
    _container: Option<Box<dyn std::any::Any + Send>>,
}

async fn sqlite() -> Backend {
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
    Backend {
        store: Arc::new(SqliteStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::SqliteSearch::new(pool)),
        _container: None,
    }
}

/// `None` when no Docker daemon answers: the Postgres half is skipped there.
async fn postgres() -> Option<Backend> {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some(Backend {
        store: Arc::new(PostgresStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::PostgresSearch::new(pool)),
        _container: Some(Box::new(container)),
    })
}

struct Harness {
    base: String,
    hooks: String,
    seen: Seen,
    state: AppState,
    store: Arc<dyn Store>,
    client: reqwest::Client,
    worker: Option<WebhookWorker>,
    _dir: tempfile::TempDir,
    _backend: Backend,
}

impl Harness {
    async fn shutdown(mut self) {
        if let Some(worker) = self.worker.take() {
            worker.shutdown().await;
        }
    }
}

/// A receiver for every hook: it records each request's path and JSON body.
async fn record(
    State(seen): State<Seen>,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, axum::Json<Value>) {
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    seen.lock().unwrap().push((uri.path().to_string(), value));
    (StatusCode::OK, axum::Json(json!({ "text": "ok" })))
}

async fn spawn(backend: Backend) -> Harness {
    spawn_with(backend, true).await
}

/// `worker`: run the webhook worker, which queues and sends deliveries on
/// its own. Without it a test queues and sends them itself.
async fn spawn_with(backend: Backend, worker: bool) -> Harness {
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    #[allow(unsafe_code)] // edition 2024 makes env writes unsafe; test setup only
    unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
    let store = backend.store.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(512)),
        backend.search.clone(),
        Arc::new(maidan_search::HashV1Provider),
        false, // auth enabled
        false,
        FederationRuntime::new(true, Some(Arc::new(KEY))),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.webhooks = WebhookRuntime::new(Some(Arc::new(KEY)));
    state.slash = SlashRuntime::new(Some(Arc::new(KEY)));
    let worker = worker.then(|| WebhookWorker::spawn(state.clone()));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let seen: Seen = Arc::default();
    let hooks_app = Router::new().fallback(record).with_state(seen.clone());
    let hooks_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hooks_addr = hooks_listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(hooks_listener, hooks_app).await.unwrap() });

    Harness {
        base: format!("http://{addr}"),
        hooks: format!("http://{hooks_addr}"),
        seen,
        state,
        store,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        worker,
        _dir: dir,
        _backend: backend,
    }
}

fn audit_event(ws: WorkspaceId, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: maidan_types::AuditScope::Workspace(ws),
        actor_id: None,
        action: action.into(),
        target_kind: None,
        target_id: None,
        metadata: json!({}),
    }
}

/// A SCIM-provisioned member with a token: one the identity provider can
/// deactivate.
struct Person {
    id: MemberId,
    handle: String,
    secret: String,
    token: ApiTokenId,
}

/// One workspace with a public channel, a thread in it, and a worker who
/// posts.
struct Case {
    ws: WorkspaceId,
    channel: ChannelId,
    thread: ThreadId,
    poster: Person,
}

impl Harness {
    fn hook(&self, name: &str) -> String {
        format!("{}/{name}", self.hooks)
    }

    /// The bodies the receiver got at `/{name}`.
    fn got(&self, name: &str) -> Vec<Value> {
        let path = format!("/{name}");
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| *p == path)
            .map(|(_, body)| body.clone())
            .collect()
    }

    /// Wait up to ten seconds for `/{name}` to get a body whose `kind` is
    /// `kind` (any body when `kind` is empty).
    async fn wait_for(&self, name: &str, kind: &str) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            if self
                .got(name)
                .iter()
                .any(|body| kind.is_empty() || body["kind"] == kind)
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }

    async fn person(&self, ws: WorkspaceId, handle: &str, set: &str) -> Person {
        let (member, _) = self
            .store
            .scim_provision_audited(
                NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                },
                Some(handle),
                true,
                Box::new(move |_| audit_event(ws, "scim.user.create")),
            )
            .await
            .unwrap();
        let secret = TokenSecret::generate();
        let token = self
            .store
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
        Person {
            id: member.id,
            handle: handle.into(),
            secret: secret.as_str().to_string(),
            token: token.id,
        }
    }

    /// Deactivate `who` through SCIM, which ends every authority it holds.
    async fn deactivate(&self, ws: WorkspaceId, who: &Person) {
        self.store
            .scim_update_user_audited(
                ws,
                who.id,
                None,
                Some(&who.handle),
                false,
                audit_event(ws, "scim.user.deactivate"),
            )
            .await
            .unwrap()
            .unwrap();
    }

    async fn case(&self, name: &str) -> Case {
        let ws = self
            .store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap()
            .id;
        let channel = self
            .store
            .create_channel(NewChannel {
                workspace_id: ws,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap()
            .id;
        let thread = self.thread(channel).await;
        let poster = self
            .person(ws, &format!("{name}-poster"), AGENT_WORKER)
            .await;
        Case {
            ws,
            channel,
            thread,
            poster,
        }
    }

    async fn thread(&self, channel: ChannelId) -> ThreadId {
        self.store
            .create_thread(NewThread {
                channel_id: channel,
                parent_thread_id: None,
                title: Some("work".into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap()
            .id
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        bearer: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let resp = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(bearer)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn post_message(&self, thread: ThreadId, bearer: &str, text: &str) -> Value {
        let (status, body) = self
            .send(
                reqwest::Method::POST,
                &format!("/threads/{}/messages", thread.0),
                bearer,
                json!({ "body": text }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "post message: {body}");
        body
    }

    async fn register_webhook(
        &self,
        ws: WorkspaceId,
        by: &Person,
        hook: &str,
        kinds: &[&str],
    ) -> String {
        let (status, body) = self
            .send(
                reqwest::Method::POST,
                &format!("/workspaces/{}/webhooks", ws.0),
                &by.secret,
                json!({ "url": self.hook(hook), "event_kinds": kinds }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "webhook: {body}");
        body["webhook"]["id"].as_str().unwrap().to_string()
    }

    async fn register_slash(&self, ws: WorkspaceId, by: &Person, name: &str) {
        let (status, body) = self
            .send(
                reqwest::Method::POST,
                &format!("/workspaces/{}/slash-commands", ws.0),
                &by.secret,
                json!({ "name": name, "handler_kind": "http", "handler_target": self.hook(name) }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "slash command: {body}");
    }

    async fn rpc(&self, bearer: &str, method: &str, params: Value) -> Value {
        self.client
            .post(format!("{}/a2a/v1/rpc", self.base))
            .bearer_auth(bearer)
            .header("A2A-Version", "1.0")
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// The audit rows `action` wrote in `ws` for `target`, waiting up to five
    /// seconds for the first.
    async fn audit(&self, ws: WorkspaceId, action: &str) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let rows: Vec<Value> = self
                .store
                .list_audit_for_workspace(ws, 200)
                .await
                .unwrap()
                .into_iter()
                .filter(|row| row.action == action)
                .map(|row| json!({ "target_id": row.target_id, "metadata": row.metadata }))
                .collect();
            if !rows.is_empty() || tokio::time::Instant::now() >= deadline {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

/// A webhook and the mention route stop with their creator; a live admin's
/// subscription keeps delivering, and only what its creator may read.
async fn webhooks_suite(backend: Backend) {
    let h = spawn(backend).await;
    let c = h.case("hooks").await;
    let leaver = h.person(c.ws, "leaver", HUMAN_ADMIN).await;
    let stayer = h.person(c.ws, "stayer", HUMAN_ADMIN).await;
    let mentioned = h.person(c.ws, "mentioned", AGENT_WORKER).await;

    let gone = h
        .register_webhook(c.ws, &leaver, "gone", &["message_posted"])
        .await;
    // Subscribed to nothing that happens here: only the mention route sends
    // it anything.
    let route = h
        .register_webhook(c.ws, &leaver, "mention-route", &["vote_cast"])
        .await;
    let (status, body) = h
        .send(
            reqwest::Method::PUT,
            &format!("/workspaces/{}/mention-webhook", c.ws.0),
            &leaver.secret,
            json!({ "webhook_id": route }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "mention route: {body}");
    h.register_webhook(
        c.ws,
        &stayer,
        "kept",
        &["message_posted", "mention_recorded"],
    )
    .await;
    assert_eq!(h.audit(c.ws, "webhook.create").await.len(), 3);
    assert_eq!(h.audit(c.ws, "mention_webhook.set").await.len(), 1);

    // A private channel the poster is in and the stayer is not.
    let private = h
        .store
        .create_channel(NewChannel {
            workspace_id: c.ws,
            name: "private".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap()
        .id;
    h.store
        .add_channel_member(private, c.poster.id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let private_thread = h.thread(private).await;

    h.deactivate(c.ws, &leaver).await;

    h.post_message(private_thread, &c.poster.secret, "not for the stayer")
        .await;
    let posted = h
        .post_message(
            c.thread,
            &c.poster.secret,
            &format!("hi @{}", mentioned.handle),
        )
        .await;
    let (status, _) = h
        .send(
            reqwest::Method::POST,
            &format!("/messages/{}/mentions", posted["id"].as_str().unwrap()),
            &c.poster.secret,
            json!({ "member_id": mentioned.id.0 }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The live admin's subscription delivers the mention, the last event, so
    // every earlier one has been through the worker by now.
    assert!(
        h.wait_for("kept", "mention_recorded").await,
        "the live admin's webhook stopped delivering"
    );
    let kept_messages = h
        .got("kept")
        .into_iter()
        .filter(|body| body["kind"] == "message_posted")
        .count();
    assert_eq!(
        kept_messages, 1,
        "a message in a private channel its creator is not in reached the webhook"
    );
    assert!(
        h.got("gone").is_empty(),
        "a deactivated admin's webhook still delivered"
    );
    assert!(
        h.got("mention-route").is_empty(),
        "a deactivated admin's mention route still delivered"
    );

    let skipped = h.audit(c.ws, "webhook.skipped").await;
    for id in [&gone, &route] {
        assert!(
            skipped.iter().any(|row| row["target_id"] == json!(id)
                && row["metadata"]["reason"] == "creator token no longer valid"),
            "no skip recorded for {id}: {skipped:?}"
        );
    }
    let listed = h.store.list_webhook_subscriptions(c.ws).await.unwrap();
    assert_eq!(listed.len(), 1, "the ended subscriptions are revoked");
    h.shutdown().await;
}

/// A delivery queued while its creator could read the event is withheld once
/// the creator has left the private channel the event happened in: a retry
/// can run long after the event. Another admin still in the channel gets it.
async fn queued_delivery_suite(backend: Backend) {
    let h = spawn_with(backend, false).await;
    let c = h.case("queued").await;
    let leaver = h.person(c.ws, "leaver", HUMAN_ADMIN).await;
    let stayer = h.person(c.ws, "stayer", HUMAN_ADMIN).await;
    let private = h
        .store
        .create_channel(NewChannel {
            workspace_id: c.ws,
            name: "private".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap()
        .id;
    for member in [c.poster.id, leaver.id, stayer.id] {
        h.store
            .add_channel_member(private, member, ChannelMemberRole::Member)
            .await
            .unwrap();
    }
    let private_thread = h.thread(private).await;
    let mut subscriptions = Vec::new();
    for (who, hook) in [(&leaver, "left"), (&stayer, "stays")] {
        let id = h
            .register_webhook(c.ws, who, hook, &["message_posted"])
            .await;
        subscriptions.push(WebhookSubscriptionId(uuid::Uuid::parse_str(&id).unwrap()));
    }
    h.post_message(private_thread, &c.poster.secret, "in private")
        .await;
    let log_id = h
        .store
        .list_events_after(c.ws, 0, 500)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.kind == EventKind::MessagePosted && e.channel_id == Some(private))
        .expect("the private message's event")
        .id;
    // Queued while both admins could read it.
    for subscription in &subscriptions {
        let _ = h
            .store
            .enqueue_webhook_delivery(*subscription, log_id, r#"{"kind":"message_posted"}"#)
            .await
            .unwrap();
    }

    h.store
        .remove_channel_member(private, leaver.id)
        .await
        .unwrap();
    webhook_worker::poll_deliveries(&h.state, 16).await.unwrap();

    assert_eq!(
        h.got("stays").len(),
        1,
        "the admin still in the channel got nothing"
    );
    assert!(
        h.got("left").is_empty(),
        "a queued delivery went to an admin who has left the channel"
    );
    assert_eq!(
        h.store
            .list_webhook_subscriptions(c.ws)
            .await
            .unwrap()
            .len(),
        2,
        "leaving a channel is not the end of the creator's authority"
    );
    h.shutdown().await;
}

/// A slash command whose creator was deactivated sends nothing to its
/// handler; a live member's still runs.
async fn slash_suite(backend: Backend) {
    let h = spawn(backend).await;
    let c = h.case("slash").await;
    let leaver = h.person(c.ws, "leaver", AGENT_WORKER).await;
    let stayer = h.person(c.ws, "stayer", AGENT_WORKER).await;
    h.register_slash(c.ws, &leaver, "gone").await;
    h.register_slash(c.ws, &stayer, "kept").await;
    assert_eq!(h.audit(c.ws, "slash_command.create").await.len(), 2);

    h.deactivate(c.ws, &leaver).await;

    let msg = h.post_message(c.thread, &c.poster.secret, "/gone hi").await;
    assert_eq!(msg["metadata"]["slash_response"]["ok"], false, "{msg}");
    assert_eq!(
        msg["metadata"]["slash_response"]["error"],
        "command_creator_not_authorized"
    );
    h.post_message(c.thread, &c.poster.secret, "/kept hi").await;
    assert!(
        h.wait_for("kept", "").await,
        "the live member's command stopped"
    );
    assert!(
        h.got("gone").is_empty(),
        "a deactivated member's command ran"
    );
    let skipped = h.audit(c.ws, "slash_command.skipped").await;
    assert_eq!(
        skipped[0]["metadata"]["reason"], "creator token no longer valid",
        "{skipped:?}"
    );
    h.shutdown().await;
}

/// An A2A push config whose creator was deactivated sends nothing and is
/// deleted; a live member's config on the same task still gets the update.
async fn a2a_push_suite(backend: Backend) {
    let h = spawn(backend).await;
    let c = h.case("push").await;
    let leaver = h.person(c.ws, "leaver", AGENT_WORKER).await;
    let stayer = h.person(c.ws, "stayer", AGENT_WORKER).await;

    let task_id = uuid::Uuid::now_v7().to_string();
    let at = chrono::Utc::now();
    let context = c.thread.0.to_string();
    h.store
        .upsert_a2a_task(A2aTaskWrite {
            workspace_id: c.ws,
            task_id: &task_id,
            context_id: Some(&context),
            thread_id: Some(c.thread),
            state: "TASK_STATE_WORKING",
            status_at: at,
            task_json: json!({
                "id": task_id,
                "contextId": context,
                "status": {
                    "state": "TASK_STATE_WORKING",
                    "timestamp": at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                },
                "metadata": { "maidan": { "threadId": c.thread.0 } },
            }),
        })
        .await
        .unwrap();
    for (who, id) in [(&leaver, "gone"), (&stayer, "kept")] {
        let resp = h
            .rpc(
                &who.secret,
                "CreateTaskPushNotificationConfig",
                json!({ "taskId": task_id, "id": id, "url": h.hook(id) }),
            )
            .await;
        assert!(resp.get("error").is_none(), "create {id}: {resp}");
    }
    assert_eq!(h.audit(c.ws, "a2a_push_config.create").await.len(), 2);

    h.deactivate(c.ws, &leaver).await;

    let resp = h
        .rpc(&c.poster.secret, "CancelTask", json!({ "id": task_id }))
        .await;
    assert!(resp.get("error").is_none(), "cancel: {resp}");
    assert!(
        h.wait_for("kept", "").await,
        "the live member's push stopped"
    );
    let skipped = h.audit(c.ws, "a2a_push_config.skipped").await;
    assert_eq!(skipped[0]["metadata"]["config_id"], "gone", "{skipped:?}");
    assert!(
        h.got("gone").is_empty(),
        "a deactivated member's push went out"
    );
    assert!(h
        .store
        .get_a2a_task_push_config(&task_id, "gone")
        .await
        .unwrap()
        .is_none());
    h.shutdown().await;
}

/// A schedule ends with its creator: paused when the creator is deactivated,
/// and skipped and paused when the creator has left its private channel.
async fn schedule_suite(backend: Backend) {
    let h = spawn(backend).await;
    let c = h.case("schedules").await;
    let leaver = h.person(c.ws, "leaver", AGENT_WORKER).await;
    let mover = h.person(c.ws, "mover", AGENT_WORKER).await;
    let stayer = h.person(c.ws, "stayer", AGENT_WORKER).await;
    let private = h
        .store
        .create_channel(NewChannel {
            workspace_id: c.ws,
            name: "private".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap()
        .id;
    h.store
        .add_channel_member(private, mover.id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let due = chrono::Utc::now() - chrono::Duration::minutes(1);
    let schedule = |channel, created_by, title: &str| NewTaskSchedule {
        workspace_id: c.ws,
        channel_id: channel,
        title: title.into(),
        interval_secs: Some(3600),
        next_run_at: due,
        created_by,
        recipe_id: None,
    };
    let by_leaver = h
        .store
        .create_task_schedule(schedule(c.channel, leaver.id, "leaver's"))
        .await
        .unwrap();
    let by_mover = h
        .store
        .create_task_schedule(schedule(private, mover.id, "mover's"))
        .await
        .unwrap();
    let by_stayer = h
        .store
        .create_task_schedule(schedule(c.channel, stayer.id, "stayer's"))
        .await
        .unwrap();

    h.deactivate(c.ws, &leaver).await;
    h.store
        .remove_channel_member(private, mover.id)
        .await
        .unwrap();

    scheduler::sweep_once(&h.state).await;

    let titles: Vec<String> = h
        .store
        .list_threads(c.channel)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|t| t.title)
        .collect();
    assert!(
        titles.iter().any(|t| t == "stayer's"),
        "the live schedule did not fire"
    );
    assert!(
        !titles.iter().any(|t| t == "leaver's"),
        "a deactivated member's schedule opened a thread"
    );
    assert!(
        h.store.list_threads(private).await.unwrap().is_empty(),
        "a schedule opened a thread in a channel its creator left"
    );
    for (id, active) in [
        (by_leaver.id, false),
        (by_mover.id, false),
        (by_stayer.id, true),
    ] {
        assert_eq!(h.store.get_task_schedule(id).await.unwrap().active, active);
    }
    let skipped = h.audit(c.ws, "task_schedule.skipped").await;
    assert!(
        skipped
            .iter()
            .any(|row| row["target_id"] == json!(by_mover.id.0)
                && row["metadata"]["reason"] == "creator is not in the channel"),
        "{skipped:?}"
    );
    h.shutdown().await;
}

/// A share ticket and an invite stop working once their owner is
/// deactivated; another member's still work.
async fn ticket_and_invite_suite(backend: Backend) {
    let h = spawn(backend).await;
    let c = h.case("shares").await;
    let leaver = h.person(c.ws, "leaver", HUMAN_ADMIN).await;
    let stayer = h.person(c.ws, "stayer", HUMAN_ADMIN).await;
    for who in [&leaver, &stayer] {
        h.store
            .create_share_ticket(NewShareTicket {
                workspace_id: c.ws,
                channel_id: c.channel,
                owner_id: who.id,
                created_by: who.id,
                token_hash: hash_secret(&format!("{}-ticket", who.handle)),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                artifact_shas: Vec::new(),
            })
            .await
            .unwrap();
        let ws = c.ws;
        h.store
            .create_workspace_invite_audited(
                NewWorkspaceInvite {
                    workspace_id: c.ws,
                    email: format!("{}-guest@example.test", who.handle),
                    role: WorkspaceInviteRole::Member,
                    created_by: who.id,
                    token_hash: hash_secret(&format!("{}-invite", who.handle)),
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                },
                Box::new(move |_| audit_event(ws, "workspace_invite.create")),
            )
            .await
            .unwrap();
    }

    h.deactivate(c.ws, &leaver).await;

    let now = chrono::Utc::now();
    for (who, live) in [(&leaver, false), (&stayer, true)] {
        let ticket = h
            .store
            .resolve_share_ticket(&hash_secret(&format!("{}-ticket", who.handle)), now)
            .await;
        assert_eq!(ticket.is_ok(), live, "{}'s ticket", who.handle);
        let invite = h
            .store
            .find_live_workspace_invite(&hash_secret(&format!("{}-invite", who.handle)), now)
            .await;
        assert_eq!(invite.is_ok(), live, "{}'s invite", who.handle);
    }
    h.shutdown().await;
}

/// A registration never acts with another workspace's token, and ending a
/// member in one workspace leaves another workspace's registrations alone.
async fn two_tenant_suite(backend: Backend) {
    let h = spawn(backend).await;
    let a = h.case("tenant-a").await;
    let b = h.case("tenant-b").await;
    let a_leaver = h.person(a.ws, "a-leaver", HUMAN_ADMIN).await;
    let a_stayer = h.person(a.ws, "a-stayer", HUMAN_ADMIN).await;
    let b_admin = h.person(b.ws, "b-admin", HUMAN_ADMIN).await;

    h.register_webhook(a.ws, &a_leaver, "a-gone", &["message_posted"])
        .await;
    h.register_webhook(a.ws, &a_stayer, "a-kept", &["message_posted"])
        .await;
    h.register_webhook(b.ws, &b_admin, "b-kept", &["message_posted"])
        .await;
    // In workspace A, naming workspace B's admin token as its creator.
    let foreign = h
        .store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: a.ws,
            url: h.hook("foreign"),
            label: None,
            event_kinds: vec!["message_posted".into()],
            secret_ciphertext: maidan_auth::encrypt_peer_secret("s", &KEY).unwrap(),
            created_by_token: Some(b_admin.token),
        })
        .await
        .unwrap();

    h.deactivate(a.ws, &a_leaver).await;

    h.post_message(a.thread, &a.poster.secret, "in a").await;
    h.post_message(b.thread, &b.poster.secret, "in b").await;
    assert!(h.wait_for("a-kept", "message_posted").await);
    assert!(
        h.wait_for("b-kept", "message_posted").await,
        "ending a member of A stopped B's webhook"
    );
    assert!(h.got("a-gone").is_empty());
    assert!(
        h.got("foreign").is_empty(),
        "a webhook delivered with another workspace's authority"
    );
    let skipped = h.audit(a.ws, "webhook.skipped").await;
    assert!(
        skipped
            .iter()
            .any(|row| row["target_id"] == json!(foreign.id.0)
                && row["metadata"]["reason"] == "creator token is for another workspace"),
        "{skipped:?}"
    );
    assert!(h.audit(b.ws, "webhook.skipped").await.is_empty());
    h.shutdown().await;
}

macro_rules! on_both_backends {
    ($suite:ident, $sqlite:ident, $postgres:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $sqlite() {
            $suite(sqlite().await).await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $postgres() {
            if let Some(backend) = postgres().await {
                $suite(backend).await;
            }
        }
    };
}

on_both_backends!(
    webhooks_suite,
    a_deactivated_admins_webhooks_stop_and_a_live_one_delivers_only_what_it_may_read_sqlite,
    a_deactivated_admins_webhooks_stop_and_a_live_one_delivers_only_what_it_may_read_postgres
);
on_both_backends!(
    queued_delivery_suite,
    a_queued_delivery_is_withheld_once_its_creator_leaves_the_private_channel_sqlite,
    a_queued_delivery_is_withheld_once_its_creator_leaves_the_private_channel_postgres
);
on_both_backends!(
    slash_suite,
    a_deactivated_members_slash_command_sends_nothing_sqlite,
    a_deactivated_members_slash_command_sends_nothing_postgres
);
on_both_backends!(
    a2a_push_suite,
    a_deactivated_members_push_config_sends_nothing_and_is_deleted_sqlite,
    a_deactivated_members_push_config_sends_nothing_and_is_deleted_postgres
);
on_both_backends!(
    schedule_suite,
    a_schedule_stops_when_its_creator_is_deactivated_or_leaves_its_channel_sqlite,
    a_schedule_stops_when_its_creator_is_deactivated_or_leaves_its_channel_postgres
);
on_both_backends!(
    ticket_and_invite_suite,
    a_deactivated_members_share_ticket_and_invite_stop_working_sqlite,
    a_deactivated_members_share_ticket_and_invite_stop_working_postgres
);
on_both_backends!(
    two_tenant_suite,
    a_registration_never_acts_with_another_workspaces_authority_sqlite,
    a_registration_never_acts_with_another_workspaces_authority_postgres
);
