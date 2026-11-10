//! The console's end-task routes: `POST /ui/api/threads/{tid}/cancel` and
//! `POST /ui/api/threads/{tid}/close-abandoned` on a browser session.
//!
//! They are the bearer handlers mounted on the session tree, so a session must
//! meet the same checks a token does: the capability first, then the creator
//! rule, and never another workspace's task.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, session::SessionSettings, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace,
    ThreadId, ThreadState, WorkspaceId,
};
use reqwest::{header, redirect::Policy, StatusCode};
use serde_json::{json, Value};

const SESSION_SECRET: &[u8] = b"test-session-secret-32-bytes-min!";

struct Tenant {
    ws: WorkspaceId,
    member: MemberId,
    channel: ChannelId,
}

struct World {
    addr: SocketAddr,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    a: Tenant,
    b: Tenant,
    _dir: tempfile::TempDir,
}

impl World {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// A session cookie holding a fresh token's authority for `t`'s member.
    async fn session(&self, t: &Tenant, caps: &[&str]) -> String {
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: t.ws,
                member_id: t.member,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: caps.iter().map(|c| c.to_string()).collect(),
                expires_at: None,
            })
            .await
            .unwrap();
        let resp = self
            .client
            .post(self.url("/auth/session/from-token"))
            .bearer_auth(secret.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "exchange");
        resp.headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("a session cookie")
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    /// A task in `t`'s channel, created by `by` (or by nobody).
    async fn task(&self, t: &Tenant, by: Option<MemberId>) -> ThreadId {
        self.store
            .create_thread(NewThread {
                channel_id: t.channel,
                parent_thread_id: None,
                title: Some("end me".into()),
                description: None,
                created_by: by,
            })
            .await
            .unwrap()
            .id
    }

    /// `POST /ui/api/threads/{tid}/{action}` on the session, from this origin.
    async fn end(
        &self,
        cookie: &str,
        tid: ThreadId,
        action: &str,
        body: Value,
    ) -> (StatusCode, String) {
        let resp = self
            .client
            .post(self.url(&format!("/ui/api/threads/{}/{action}", tid.0)))
            .header(header::COOKIE, cookie)
            .header(header::ORIGIN, format!("http://{}", self.addr))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.text().await.unwrap())
    }

    async fn state(&self, tid: ThreadId) -> ThreadState {
        self.store.get_thread(tid).await.unwrap().state
    }
}

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-human"),
            display_name: None,
            kind: MemberKind::Human,
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
    Tenant {
        ws: ws.id,
        member: member.id,
        channel: channel.id,
    }
}

async fn world() -> World {
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
    let a = tenant(store.as_ref(), "a").await;
    let b = tenant(store.as_ref(), "b").await;
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
    state.sessions = Some(SessionSettings {
        secret: Arc::from(SESSION_SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    World {
        addr,
        client: reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        a,
        b,
        _dir: dir,
    }
}

/// What a person signed in to the console holds.
const MEMBER: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::MESSAGE_POST,
    capability::THREAD_TRANSITION,
];

#[tokio::test]
async fn a_session_member_cancels_one_task_and_closes_another_as_abandoned() {
    let w = world().await;
    let cookie = w.session(&w.a, MEMBER).await;

    // Not the creator: `thread:transition` reaches any task in the workspace.
    let cancel_me = w.task(&w.a, None).await;
    let (status, body) = w
        .end(
            &cookie,
            cancel_me,
            "cancel",
            json!({ "reason": "nobody needs it" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let out: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(out["thread"]["state"], "closed", "{body}");
    assert_eq!(out["replayed"], false, "{body}");
    assert_eq!(w.state(cancel_me).await, ThreadState::Closed);

    // A second press answers with the first cancel and changes nothing.
    let (status, body) = w
        .end(&cookie, cancel_me, "cancel", json!({ "reason": "again" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let again: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(again["replayed"], true, "{body}");

    // The creator closes a task with no result: no review gate applies.
    let abandon_me = w.task(&w.a, Some(w.a.member)).await;
    let (status, body) = w
        .end(
            &cookie,
            abandon_me,
            "close-abandoned",
            json!({ "reason": "abandoned" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let closed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(closed["state"], "closed", "{body}");
    assert_eq!(w.state(abandon_me).await, ThreadState::Closed);

    // An ended task has nothing left to close.
    let (status, body) = w
        .end(
            &cookie,
            abandon_me,
            "close-abandoned",
            json!({ "reason": "abandoned" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn another_tenants_session_cannot_end_a_task() {
    let w = world().await;
    let intruder = w.session(&w.b, MEMBER).await;
    let task = w.task(&w.a, Some(w.a.member)).await;

    for (action, body) in [
        ("cancel", json!({ "reason": "not yours" })),
        ("close-abandoned", json!({ "reason": "abandoned" })),
    ] {
        let (status, text) = w.end(&intruder, task, action, body).await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "{action}: {status} {text}"
        );
        assert!(!text.contains("end me"), "{action} leaked the task: {text}");
    }
    assert_eq!(w.state(task).await, ThreadState::Open);
}

#[tokio::test]
async fn a_session_member_without_the_capability_is_refused() {
    let w = world().await;
    let mine = w.task(&w.a, Some(w.a.member)).await;
    let theirs = w.task(&w.a, None).await;

    // Read-only: neither route.
    let reader = w.session(&w.a, &[capability::WORKSPACE_READ]).await;
    let (status, text) = w
        .end(&reader, mine, "cancel", json!({ "reason": "no" }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        text.contains("missing capability: workspace:write"),
        "{text}"
    );

    // `workspace:write` without `thread:transition`: no close-abandoned at
    // all, and a cancel only of a task this member created.
    let writer = w
        .session(
            &w.a,
            &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE],
        )
        .await;
    let (status, text) = w
        .end(
            &writer,
            mine,
            "close-abandoned",
            json!({ "reason": "abandoned" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        text.contains("missing capability: thread:transition"),
        "{text}"
    );
    let (status, text) = w
        .end(&writer, theirs, "cancel", json!({ "reason": "not mine" }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert_eq!(w.state(mine).await, ThreadState::Open);
    assert_eq!(w.state(theirs).await, ThreadState::Open);

    let (status, text) = w
        .end(&writer, mine, "cancel", json!({ "reason": "mine to drop" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(w.state(mine).await, ThreadState::Closed);
}
