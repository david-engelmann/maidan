//! Opening an approval gate and reading one by id over REST, the twins of MCP
//! `request_approval` and `get_approval_gate`, and the `approval_resolved`
//! event every answer appends. Auth ENABLED, two workspaces.
//!
//! - `POST /workspaces/:wid/approval-gates` needs `message:post`, opens a
//!   pending gate attributed to the caller, and refuses another workspace or
//!   a thread the caller cannot read (403, as MCP's id-scope guard does)
//!   without writing anything.
//! - `GET /approval-gates/:id` is 404 for a gate in another workspace or on a
//!   thread the caller cannot read.
//! - Nobody accepts a gate they opened here.
//! - An answer reaches the live subscribers who can read the gate, once: not
//!   another workspace, and not a member outside the gate's private channel.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::SinkExt;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelMemberRole, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread,
    NewWorkspace, ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: &[&str]) -> String {
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

async fn spawn() -> (SocketAddr, Arc<dyn Store>) {
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    // Signs the subscribe ack and each gate's `request_state`.
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, store)
}

const POSTER: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::MESSAGE_POST,
    capability::EVENT_SUBSCRIBE,
];

/// Two workspaces. In `home`: `alice` and `bob` share a private channel that
/// `carol` is not in, and all three can read an open channel; `bob` alone
/// holds `approval:grant`. In `away`: `eve`.
struct World {
    addr: SocketAddr,
    store: Arc<dyn Store>,
    home: WorkspaceId,
    away: WorkspaceId,
    alice: MemberId,
    bob: MemberId,
    open_thread: ThreadId,
    private_thread: ThreadId,
    private_channel: maidan_types::ChannelId,
    alice_tok: String,
    alice_reader: String,
    bob_tok: String,
    carol_tok: String,
    eve_tok: String,
    http: reqwest::Client,
}

async fn world() -> World {
    let (addr, store) = spawn().await;
    let home = store
        .create_workspace(NewWorkspace {
            name: "home".into(),
        })
        .await
        .unwrap()
        .id;
    let away = store
        .create_workspace(NewWorkspace {
            name: "away".into(),
        })
        .await
        .unwrap()
        .id;
    let person = |ws: WorkspaceId, handle: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
                .id
        }
    };
    let alice = person(home, "alice").await;
    let bob = person(home, "bob").await;
    let carol = person(home, "carol").await;
    let eve = person(away, "eve").await;
    let channel = |name: &'static str, private: bool| {
        let store = store.clone();
        async move {
            let channel = store
                .create_channel(NewChannel {
                    workspace_id: home,
                    name: name.into(),
                    topic: None,
                    private,
                })
                .await
                .unwrap();
            let thread = store
                .create_thread(NewThread {
                    channel_id: channel.id,
                    parent_thread_id: None,
                    title: Some(name.into()),
                    description: None,
                    created_by: None,
                })
                .await
                .unwrap();
            (channel.id, thread.id)
        }
    };
    let (_, open_thread) = channel("general", false).await;
    let (private_channel, private_thread) = channel("board", true).await;
    for m in [alice, bob] {
        store
            .add_channel_member(private_channel, m, ChannelMemberRole::Member)
            .await
            .unwrap();
    }
    let mut granter = POSTER.to_vec();
    granter.push(capability::APPROVAL_GRANT);
    World {
        alice_tok: mint(store.as_ref(), home, alice, POSTER).await,
        alice_reader: mint(store.as_ref(), home, alice, &[capability::WORKSPACE_READ]).await,
        bob_tok: mint(store.as_ref(), home, bob, &granter).await,
        carol_tok: mint(store.as_ref(), home, carol, POSTER).await,
        eve_tok: mint(store.as_ref(), away, eve, POSTER).await,
        addr,
        store,
        home,
        away,
        alice,
        bob,
        open_thread,
        private_thread,
        private_channel,
        http: reqwest::Client::new(),
    }
}

impl World {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn open(&self, token: &str, ws: WorkspaceId, body: Value) -> reqwest::Response {
        self.http
            .post(self.url(&format!("/workspaces/{}/approval-gates", ws.0)))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    async fn get(&self, token: &str, gate: &str) -> reqwest::Response {
        self.http
            .get(self.url(&format!("/approval-gates/{gate}")))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }

    /// The `request_state` `token`'s listing hands out for `gate`.
    async fn request_state(&self, token: &str, gate: &str) -> String {
        let views: Vec<Value> = self
            .http
            .get(self.url(&format!("/workspaces/{}/approval-gates", self.home.0)))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        views
            .iter()
            .find(|v| v["gate"]["id"] == gate)
            .and_then(|v| v["request_state"].as_str())
            .unwrap_or_else(|| panic!("{gate} is not in the listing: {views:?}"))
            .to_string()
    }

    async fn answer(&self, token: &str, gate: &str, action: &str) -> reqwest::Response {
        let request_state = self.request_state(token, gate).await;
        self.http
            .post(self.url(&format!("/approval-gates/{gate}/answer")))
            .bearer_auth(token)
            .json(&json!({ "action": action, "request_state": request_state }))
            .send()
            .await
            .unwrap()
    }

    async fn pending(&self, ws: WorkspaceId) -> usize {
        self.store
            .list_pending_approval_gates(ws, None, 50)
            .await
            .unwrap()
            .len()
    }

    /// A live subscription for `token` that asserts a grant for the private
    /// channel: the server keeps it only for a member of that channel.
    async fn subscribe(
        &self,
        token: &str,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let url = format!("ws://{}/ws/subscribe", self.addr);
        let (mut ws, _) = connect_async(url.into_client_request().unwrap())
            .await
            .unwrap();
        ws.send(Message::Text(
            json!({
                "token": token,
                "filter": { "channel_grants": [self.private_channel.0] },
            })
            .to_string(),
        ))
        .await
        .unwrap();
        drain(&mut ws, Duration::from_millis(300)).await;
        ws
    }
}

async fn drain<S>(ws: &mut S, window: Duration) -> Vec<Value>
where
    S: futures::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    use futures::StreamExt;
    let mut frames = Vec::new();
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, ws.next()).await {
        if let Message::Text(text) = msg
            && let Ok(frame) = serde_json::from_str(&text)
        {
            frames.push(frame);
        }
    }
    frames
}

fn of_kind<'a>(frames: &'a [Value], kind: &str) -> Vec<&'a Value> {
    frames.iter().filter(|f| f["kind"] == kind).collect()
}

/// A gate opened over REST is the caller's, pending, and read back by id;
/// another workspace can neither read it nor open one here, and a read-only
/// token opens nothing. Every refusal leaves the gate count where it was.
#[tokio::test]
async fn a_gate_opened_over_rest_reads_back_by_id_and_no_other_workspace_reaches_it() {
    let w = world().await;

    let refused = w
        .open(&w.alice_reader, w.home, json!({ "prompt": "ship v2?" }))
        .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(w.pending(w.home).await, 0, "a refused open wrote a gate");

    let opened = w
        .open(
            &w.alice_tok,
            w.home,
            json!({ "prompt": "ship v2?", "thread_id": w.open_thread.0, "risk": "low" }),
        )
        .await;
    assert_eq!(opened.status(), StatusCode::CREATED);
    let gate: Value = opened.json().await.unwrap();
    let id = gate["id"].as_str().unwrap().to_string();
    assert_eq!(gate["state"], "pending");
    assert_eq!(gate["requested_by"], w.alice.0.to_string());
    assert_eq!(gate["workspace_id"], w.home.0.to_string());
    assert_eq!(gate["thread_id"], w.open_thread.0.to_string());
    assert_eq!(gate["risk"], "low");

    let read = w.get(&w.alice_reader, &id).await;
    assert_eq!(read.status(), StatusCode::OK);
    let read: Value = read.json().await.unwrap();
    assert_eq!(read["id"], id);
    assert_eq!(read["prompt"], "ship v2?");
    assert_eq!(read["state"], "pending");

    // Another workspace: not found by id, refused at the path, and refused
    // when it names this workspace's thread from its own.
    assert_eq!(w.get(&w.eve_tok, &id).await.status(), StatusCode::NOT_FOUND);
    let crossed = w
        .open(&w.eve_tok, w.home, json!({ "prompt": "theirs?" }))
        .await;
    assert_eq!(crossed.status(), StatusCode::FORBIDDEN);
    let borrowed = w
        .open(
            &w.eve_tok,
            w.away,
            json!({ "prompt": "hold their thread?", "thread_id": w.open_thread.0 }),
        )
        .await;
    assert_eq!(borrowed.status(), StatusCode::FORBIDDEN);
    assert_eq!(w.pending(w.home).await, 1);
    assert_eq!(w.pending(w.away).await, 0);

    // The body names no requester, and a misspelled field is not ignored.
    let named = w
        .open(
            &w.alice_tok,
            w.home,
            json!({ "prompt": "on bob's behalf", "requested_by": w.bob.0 }),
        )
        .await;
    assert_eq!(named.status(), StatusCode::BAD_REQUEST);
    let misspelled = w
        .open(
            &w.alice_tok,
            w.home,
            json!({ "prompt": "hold it", "threadId": w.open_thread.0 }),
        )
        .await;
    assert_eq!(misspelled.status(), StatusCode::BAD_REQUEST);
    assert_eq!(w.pending(w.home).await, 1);

    let unknown = uuid::Uuid::now_v7().to_string();
    assert_eq!(
        w.get(&w.alice_tok, &unknown).await.status(),
        StatusCode::NOT_FOUND
    );
}

/// A gate holds its thread, so only someone who can read the thread opens or
/// reads one there: a member outside a private channel is refused the open
/// (403, as over MCP) and gets 404 for the gate, and nothing is written.
#[tokio::test]
async fn a_gate_on_a_private_thread_is_opened_and_read_only_by_its_participants() {
    let w = world().await;
    let body = json!({ "prompt": "send the deck?", "thread_id": w.private_thread.0 });

    let refused = w.open(&w.carol_tok, w.home, body.clone()).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let unknown = w
        .open(
            &w.alice_tok,
            w.home,
            json!({ "prompt": "hold a ghost?", "thread_id": uuid::Uuid::now_v7() }),
        )
        .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_eq!(w.pending(w.home).await, 0, "a refused open wrote a gate");

    let opened = w.open(&w.alice_tok, w.home, body).await;
    assert_eq!(opened.status(), StatusCode::CREATED);
    let id = opened.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(w.get(&w.alice_tok, &id).await.status(), StatusCode::OK);
    assert_eq!(w.get(&w.bob_tok, &id).await.status(), StatusCode::OK);
    assert_eq!(
        w.get(&w.carol_tok, &id).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(w.get(&w.eve_tok, &id).await.status(), StatusCode::NOT_FOUND);
}

/// Opening a gate over REST is no way around the rule that nobody approves
/// their own request: the opener's accept is refused even with
/// `approval:grant`, and the gate stays pending.
#[tokio::test]
async fn the_member_who_opened_a_gate_over_rest_cannot_accept_it() {
    let w = world().await;
    let opened = w
        .open(
            &w.bob_tok,
            w.home,
            json!({ "prompt": "approve my own change?" }),
        )
        .await;
    assert_eq!(opened.status(), StatusCode::CREATED);
    let id = opened.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let accepted = w.answer(&w.bob_tok, &id, "accept").await;
    assert_eq!(accepted.status(), StatusCode::FORBIDDEN);
    let read: Value = w.get(&w.bob_tok, &id).await.json().await.unwrap();
    assert_eq!(read["state"], "pending");
    assert_eq!(w.pending(w.home).await, 1);
}

/// Each answer is announced once as `approval_resolved`, naming the gate,
/// its state, who asked and who answered. A gate on a private thread reaches
/// only that channel's members; another workspace hears nothing; a refused
/// second answer announces nothing.
#[tokio::test]
async fn answering_a_gate_announces_approval_resolved_only_to_those_who_can_read_it() {
    let w = world().await;
    let mut alice_ws = w.subscribe(&w.alice_tok).await;
    let mut carol_ws = w.subscribe(&w.carol_tok).await;
    let mut eve_ws = w.subscribe(&w.eve_tok).await;

    let open = |body: Value| async {
        let resp = w.open(&w.alice_tok, w.home, body).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        resp.json::<Value>().await.unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let private =
        open(json!({ "prompt": "send the deck?", "thread_id": w.private_thread.0 })).await;
    let standalone = open(json!({ "prompt": "pause deploys?" })).await;

    // bob holds approval:grant and did not ask: he accepts one and declines
    // the other.
    assert_eq!(
        w.answer(&w.bob_tok, &private, "accept").await.status(),
        StatusCode::OK
    );
    let standalone_state = w.request_state(&w.bob_tok, &standalone).await;
    let answer_standalone = |action: &'static str| {
        w.http
            .post(w.url(&format!("/approval-gates/{standalone}/answer")))
            .bearer_auth(&w.bob_tok)
            .json(&json!({ "action": action, "request_state": standalone_state }))
            .send()
    };
    assert_eq!(
        answer_standalone("decline").await.unwrap().status(),
        StatusCode::OK
    );
    // A second answer is a conflict, and says nothing on the stream.
    assert_eq!(
        answer_standalone("cancel").await.unwrap().status(),
        StatusCode::CONFLICT
    );

    let alice = drain(&mut alice_ws, Duration::from_millis(800)).await;
    let requested = of_kind(&alice, "approval_requested");
    assert_eq!(requested.len(), 2, "alice hears both gates open: {alice:?}");
    let resolved = of_kind(&alice, "approval_resolved");
    assert_eq!(resolved.len(), 2, "one event per answer: {alice:?}");
    let on_private = resolved
        .iter()
        .find(|f| f["gate_id"] == private)
        .expect("the private gate's answer");
    assert_eq!(on_private["state"], "accepted");
    assert_eq!(on_private["thread_id"], w.private_thread.0.to_string());
    assert_eq!(on_private["channel_id"], w.private_channel.0.to_string());
    assert_eq!(on_private["requested_by"], w.alice.0.to_string());
    assert_eq!(on_private["resolved_by"], w.bob.0.to_string());
    let on_standalone = resolved
        .iter()
        .find(|f| f["gate_id"] == standalone)
        .expect("the standalone gate's answer");
    assert_eq!(on_standalone["state"], "declined");
    assert!(on_standalone.get("thread_id").is_none_or(Value::is_null));

    // carol is in the workspace but not the private channel, so her asserted
    // grant was dropped: she hears the workspace-level gate and nothing of the
    // private one.
    let carol = drain(&mut carol_ws, Duration::from_millis(300)).await;
    assert!(
        carol.iter().all(|f| f["gate_id"] != private),
        "a non-participant heard the private gate: {carol:?}"
    );
    assert_eq!(of_kind(&carol, "approval_resolved").len(), 1);

    let eve = drain(&mut eve_ws, Duration::from_millis(300)).await;
    assert!(
        of_kind(&eve, "approval_resolved").is_empty()
            && of_kind(&eve, "approval_requested").is_empty(),
        "another workspace heard this workspace's gates: {eve:?}"
    );
    let logged = w
        .store
        .list_events_after(w.away, 0, 500)
        .await
        .unwrap()
        .iter()
        .filter(|e| e.kind == maidan_types::EventKind::ApprovalResolved)
        .count();
    assert_eq!(logged, 0);
}
