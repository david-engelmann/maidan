//! WS subscribe channel grants, and what a live stream shows a member: one
//! rule per event, decided when it is sent, so a grant never lends access.

use std::{collections::VecDeque, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, subscribe_grants, test_support, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::MemberKind;
use serde_json::{json, Value};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

async fn spawn() -> (
    std::net::SocketAddr,
    test_support::Tenant,
    tokio::task::JoinHandle<()>,
) {
    let rig = rig().await;
    (rig.addr, rig.tenant, rig.server)
}

struct Rig {
    addr: std::net::SocketAddr,
    store: Arc<dyn Store>,
    tenant: test_support::Tenant,
    server: tokio::task::JoinHandle<()>,
}

async fn rig() -> Rig {
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
    let tenant = test_support::tenant(store.as_ref(), "grants-ws").await;
    let kept_store = store.clone();
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Rig {
        addr,
        store: kept_store,
        tenant,
        server,
    }
}

#[tokio::test]
async fn subscribe_denies_private_channel_without_grant() {
    let (addr, tenant, server) = spawn().await;
    let client = tenant.client();
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");
    let workspace_id = tenant.workspace_id.0.to_string();

    let req = ws_url.clone().into_client_request().unwrap();
    let (mut socket, _resp) = connect_async(req).await.unwrap();
    socket
        .send(Message::Text(
            json!({
                "token": tenant.token,
                "filter": {
                    "workspace_id": workspace_id,
                    "channel_grants": []
                }
            })
            .to_string(),
        ))
        .await
        .unwrap();

    let mut past_ack = false;
    let ack_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < ack_deadline {
        let Some(Ok(Message::Text(payload))) = socket.next().await else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        if v.get("type").and_then(|t| t.as_str()) == Some("subscribe_ack") {
            past_ack = true;
            break;
        }
    }
    assert!(past_ack);

    let public_ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "public", "private": false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let public_id = public_ch["id"].as_str().unwrap();

    let private_ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "secret", "private": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let private_id = private_ch["id"].as_str().unwrap();

    let mut saw_public = false;
    let mut saw_private = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && (!saw_public || !saw_private) {
        let Ok(Some(Ok(Message::Text(payload)))) =
            tokio::time::timeout(Duration::from_millis(500), socket.next()).await
        else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        if v.get("type").is_some() {
            continue;
        }
        if v.get("kind").and_then(|k| k.as_str()) == Some("channel_created") {
            let ch = v["channel"]["id"].as_str().unwrap();
            if ch == private_id {
                saw_private = true;
            }
            if ch == public_id {
                saw_public = true;
            }
        }
    }
    assert!(saw_public, "expected public channel_created");
    assert!(
        !saw_private,
        "must not receive private channel without grant"
    );

    let req2 = ws_url.clone().into_client_request().unwrap();
    let (mut socket2, _) = connect_async(req2).await.unwrap();
    socket2
        .send(Message::Text(
            json!({
                "token": tenant.token,
                "filter": {
                    "workspace_id": workspace_id,
                    "channel_id": private_id
                }
            })
            .to_string(),
        ))
        .await
        .unwrap();
    let close = tokio::time::timeout(Duration::from_secs(2), socket2.next())
        .await
        .expect("expected close")
        .expect("stream ended")
        .expect("ws frame");
    match close {
        Message::Close(frame) => {
            let code: u16 = frame.map(|f| f.code.into()).unwrap_or(1000);
            assert_eq!(code, 1008);
        }
        other => panic!("expected close frame, got {other:?}"),
    }

    server.abort();
}

#[tokio::test]
async fn subscribe_grants_helper_builds_filter() {
    let ws = maidan_types::WorkspaceId(uuid::Uuid::new_v4());
    let ch = maidan_types::ChannelId(uuid::Uuid::new_v4());
    let filter = subscribe_grants::workspace_filter(ws, &[ch]);
    assert_eq!(filter.workspace_id, Some(ws));
    assert_eq!(filter.channel_grants.as_deref(), Some(&[ch][..]));
}

// What a live stream shows a member. Each test posts a public sentinel after
// the activity it is about, and reads until the sentinel arrives: the stream
// delivers in order, so anything that was going to leak has arrived by then.

/// How long a stream reuses its answer about one thread or channel, plus slack.
const PAST_VISIBILITY_LIFETIME: Duration = Duration::from_millis(5_600);

/// Event frames of one subscription, with control frames set aside.
struct Feed {
    rx: tokio::sync::mpsc::UnboundedReceiver<Value>,
    buffered: VecDeque<Value>,
    resume_token: String,
    _reader: tokio::task::JoinHandle<()>,
}

impl Feed {
    /// Reads until `stop` accepts a frame and returns every event frame up to
    /// and including it.
    async fn until(&mut self, stop: impl Fn(&Value) -> bool) -> Vec<Value> {
        let mut seen = Vec::new();
        let work = async {
            loop {
                let frame = match self.buffered.pop_front() {
                    Some(frame) => frame,
                    None => self.rx.recv().await.expect("stream ended early"),
                };
                // Control frames carry a `type`; event frames never do.
                if frame.get("type").is_some() {
                    continue;
                }
                let done = stop(&frame);
                seen.push(frame);
                if done {
                    break;
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(10), work)
            .await
            .expect("timed out waiting for the stop frame");
        seen
    }

    async fn until_body(&mut self, body: &str) -> Vec<Value> {
        self.until(|f| body_of(f) == Some(body)).await
    }
}

fn body_of(frame: &Value) -> Option<&str> {
    frame["message"]["body"].as_str()
}

fn bodies(frames: &[Value]) -> Vec<&str> {
    frames.iter().filter_map(body_of).collect()
}

/// Whether any frame names `id` anywhere, as a message, thread or channel.
fn mentions_id(frames: &[Value], id: &str) -> bool {
    frames.iter().any(|f| f.to_string().contains(id))
}

async fn ws_feed(addr: std::net::SocketAddr, token: &str, filter: Value, after_id: i64) -> Feed {
    let req = format!("ws://{addr}/ws/subscribe")
        .into_client_request()
        .unwrap();
    let (mut socket, _) = connect_async(req).await.unwrap();
    socket
        .send(Message::Text(
            json!({"token": token, "filter": filter, "after_id": after_id}).to_string(),
        ))
        .await
        .unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        while let Some(Ok(frame)) = socket.next().await {
            if let Message::Text(text) = frame
                && let Ok(value) = serde_json::from_str::<Value>(&text)
                && tx.send(value).is_err()
            {
                break;
            }
        }
    });
    ready(rx, reader).await
}

async fn mcp_feed(addr: std::net::SocketAddr, token: &str, query: &str) -> Feed {
    let response = reqwest::Client::new()
        .get(format!("http://{addr}/mcp/stream?{query}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "the stream opens");
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        let mut bytes = response.bytes_stream();
        let mut buf = String::new();
        while let Some(Ok(chunk)) = bytes.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = buf.find("\n\n") {
                let block: String = buf.drain(..end + 2).collect();
                for line in block.lines() {
                    if let Some(data) = line.strip_prefix("data:")
                        && let Ok(value) = serde_json::from_str::<Value>(data.trim())
                        && tx.send(value).is_err()
                    {
                        return;
                    }
                }
            }
        }
    });
    ready(rx, reader).await
}

/// Waits for the `subscribe_ack`, so the caller knows the stream is live. Event
/// frames that arrive first (a replay) are kept for the first read.
async fn ready(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Value>,
    reader: tokio::task::JoinHandle<()>,
) -> Feed {
    let mut buffered = VecDeque::new();
    let resume_token = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let frame = rx.recv().await.expect("stream ended before the ack");
            match frame["type"].as_str() {
                Some("subscribe_ack") => {
                    break frame["resume_token"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string()
                }
                Some(_) => {}
                None => buffered.push_back(frame),
            }
        }
    })
    .await
    .expect("no subscribe_ack");
    Feed {
        rx,
        buffered,
        resume_token,
        _reader: reader,
    }
}

/// A workspace with an admin, three members and one public channel with a
/// thread. Alice and Bob are the ones who talk; Carol is the outsider.
struct Cast {
    rig: Rig,
    admin: reqwest::Client,
    base: String,
    ws: String,
    alice: test_support::Caller,
    bob: test_support::Caller,
    carol: test_support::Caller,
    public_channel: String,
    public_thread: String,
}

async fn cast() -> Cast {
    let rig = rig().await;
    let admin = rig.tenant.client();
    let base = format!("http://{}", rig.addr);
    let ws = rig.tenant.workspace_id.0.to_string();
    let store = rig.store.as_ref();
    let alice = rig.tenant.member(store, "alice", MemberKind::Human).await;
    let bob = rig.tenant.member(store, "bob", MemberKind::Human).await;
    let carol = rig.tenant.member(store, "carol", MemberKind::Human).await;
    let mut cast = Cast {
        rig,
        admin,
        base,
        ws,
        alice,
        bob,
        carol,
        public_channel: String::new(),
        public_thread: String::new(),
    };
    cast.public_channel = cast.channel("lobby", false).await;
    cast.public_thread = cast.thread(&cast.public_channel).await;
    cast
}

impl Cast {
    async fn created(&self, response: reqwest::Response) -> Value {
        let status = response.status();
        let body = response.text().await.unwrap();
        assert!(status.is_success(), "request failed: {status} {body}");
        serde_json::from_str(&body).unwrap_or(Value::Null)
    }

    async fn channel(&self, name: &str, private: bool) -> String {
        let response = self
            .admin
            .post(format!("{}/workspaces/{}/channels", self.base, self.ws))
            .json(&json!({"name": name, "private": private}))
            .send()
            .await
            .unwrap();
        self.created(response).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn thread(&self, channel: &str) -> String {
        let response = self
            .admin
            .post(format!("{}/channels/{channel}/threads", self.base))
            .json(&json!({"title": "topic"}))
            .send()
            .await
            .unwrap();
        self.created(response).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn add_to_channel(&self, channel: &str, member: &test_support::Caller) {
        let response = self
            .admin
            .post(format!("{}/channels/{channel}/members", self.base))
            .json(&json!({"member_id": member.member_id.0}))
            .send()
            .await
            .unwrap();
        self.created(response).await;
    }

    async fn say(&self, client: &reqwest::Client, thread: &str, body: &str) -> String {
        let response = client
            .post(format!("{}/threads/{thread}/messages", self.base))
            .json(&json!({"body": body}))
            .send()
            .await
            .unwrap();
        self.created(response).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// A public message every subscriber to the lobby receives, posted after
    /// the activity under test so reading up to it reads all of that activity.
    async fn sentinel(&self, label: &str) -> String {
        self.say(&self.admin, &self.public_thread, label).await;
        label.to_string()
    }

    /// Opens a DM between two members. Returns its id and the DM channel id.
    async fn open_dm(
        &self,
        from: &test_support::Caller,
        to: &test_support::Caller,
    ) -> (String, String) {
        let response = from
            .client()
            .post(format!("{}/workspaces/{}/dm", self.base, self.ws))
            .json(&json!({"other_member_id": to.member_id.0}))
            .send()
            .await
            .unwrap();
        let dm = self.created(response).await;
        let channel = self
            .rig
            .store
            .list_channels(self.rig.tenant.workspace_id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.name == maidan_types::DM_CHANNEL_NAME)
            .expect("the DM channel exists once a DM is open");
        (
            dm["id"].as_str().unwrap().to_string(),
            channel.id.0.to_string(),
        )
    }

    async fn dm_say(&self, from: &test_support::Caller, dm: &str, body: &str) {
        let response = from
            .client()
            .post(format!("{}/dm/{dm}/messages", self.base))
            .json(&json!({"body": body}))
            .send()
            .await
            .unwrap();
        self.created(response).await;
    }

    /// A workspace-wide subscription that names these channels in its grants.
    fn granting(&self, channels: &[&str]) -> Value {
        json!({"workspace_id": self.ws, "channel_grants": channels})
    }

    fn everything(&self) -> Value {
        json!({"workspace_id": self.ws})
    }

    /// A message in a private channel with a reaction, a vote and a mention on
    /// it. Returns the thread id.
    async fn private_activity(&self, channel: &str, mention: &test_support::Caller) -> String {
        let thread = self.thread(channel).await;
        let message = self.say(&self.admin, &thread, "private-words").await;
        let respond = |path: &str, body: Value| {
            let url = format!("{}/messages/{message}/{path}", self.base);
            let request = self.admin.post(url).json(&body);
            async move { request.send().await.unwrap() }
        };
        for (path, body) in [
            ("reactions", json!({"emoji": "🎉"})),
            ("votes", json!({"kind": "ack", "confidence": 0.9})),
            ("mentions", json!({"member_id": mention.member_id.0})),
        ] {
            self.created(respond(path, body).await).await;
        }
        thread
    }
}

fn kinds(frames: &[Value]) -> Vec<&str> {
    frames.iter().filter_map(|f| f["kind"].as_str()).collect()
}

#[tokio::test]
async fn a_member_outside_a_dm_gets_none_of_its_messages_over_ws_when_it_names_the_dm_channel() {
    let c = cast().await;
    let (dm, dm_channel) = c.open_dm(&c.alice, &c.bob).await;
    let filter = c.granting(&[&dm_channel, &c.public_channel]);
    let mut carol = ws_feed(c.rig.addr, &c.carol.token, filter.clone(), 0).await;
    let mut bob = ws_feed(c.rig.addr, &c.bob.token, filter, 0).await;

    c.dm_say(&c.alice, &dm, "dm-live").await;
    let end = c.sentinel("end-live").await;

    let carol_saw = carol.until_body(&end).await;
    assert!(
        !bodies(&carol_saw).contains(&"dm-live"),
        "an outsider receives the DM's words: {carol_saw:?}"
    );
    let bob_saw = bob.until_body(&end).await;
    assert!(
        bodies(&bob_saw).contains(&"dm-live"),
        "a participant keeps receiving its DM: {bob_saw:?}"
    );
    c.rig.server.abort();
}

#[tokio::test]
async fn a_member_outside_a_dm_gets_none_of_its_messages_on_ws_replay() {
    let c = cast().await;
    let (dm, dm_channel) = c.open_dm(&c.alice, &c.bob).await;
    c.dm_say(&c.alice, &dm, "dm-before-subscribe").await;
    let end = c.sentinel("end-replay").await;
    let filter = c.granting(&[&dm_channel, &c.public_channel]);

    let mut carol = ws_feed(c.rig.addr, &c.carol.token, filter.clone(), 1).await;
    let mut bob = ws_feed(c.rig.addr, &c.bob.token, filter, 1).await;

    let carol_saw = carol.until_body(&end).await;
    assert!(
        !bodies(&carol_saw).contains(&"dm-before-subscribe"),
        "an outsider replays the DM's words: {carol_saw:?}"
    );
    let bob_saw = bob.until_body(&end).await;
    assert!(bodies(&bob_saw).contains(&"dm-before-subscribe"));
    c.rig.server.abort();
}

#[tokio::test]
async fn a_member_outside_a_dm_gets_none_of_its_messages_over_mcp_stream_live_or_replayed() {
    let c = cast().await;
    let (dm, dm_channel) = c.open_dm(&c.alice, &c.bob).await;
    let filter = c.granting(&[&dm_channel, &c.public_channel]);
    // The grant reaches `/mcp/stream` inside a resume token, which carries the
    // filter its subscribe frame asked for.
    let carol_token = ws_feed(c.rig.addr, &c.carol.token, filter.clone(), 0)
        .await
        .resume_token;
    let bob_token = ws_feed(c.rig.addr, &c.bob.token, filter, 0)
        .await
        .resume_token;

    c.dm_say(&c.alice, &dm, "dm-replayed").await;
    let first_end = c.sentinel("end-first").await;
    let mut carol = mcp_feed(
        c.rig.addr,
        &c.carol.token,
        &format!("resume_token={carol_token}"),
    )
    .await;
    let mut bob = mcp_feed(
        c.rig.addr,
        &c.bob.token,
        &format!("resume_token={bob_token}"),
    )
    .await;
    c.dm_say(&c.alice, &dm, "dm-live").await;
    let second_end = c.sentinel("end-second").await;

    let carol_saw = [
        carol.until_body(&first_end).await,
        carol.until_body(&second_end).await,
    ]
    .concat();
    assert!(
        !bodies(&carol_saw).iter().any(|b| b.starts_with("dm-")),
        "an outsider receives the DM's words: {carol_saw:?}"
    );
    let bob_saw = [
        bob.until_body(&first_end).await,
        bob.until_body(&second_end).await,
    ]
    .concat();
    let bob_bodies = bodies(&bob_saw);
    assert!(bob_bodies.contains(&"dm-replayed") && bob_bodies.contains(&"dm-live"));
    c.rig.server.abort();
}

#[tokio::test]
async fn both_dm_participants_receive_it_on_a_dm_conversation_subscription() {
    let c = cast().await;
    let (dm, _) = c.open_dm(&c.alice, &c.bob).await;
    let filter = json!({"workspace_id": c.ws, "dm_conversation_id": dm});
    let mut alice_ws = ws_feed(c.rig.addr, &c.alice.token, filter.clone(), 0).await;
    let mut bob_ws = ws_feed(c.rig.addr, &c.bob.token, filter, 0).await;
    let query = format!("workspace_id={}&dm_conversation_id={dm}", c.ws);
    let mut alice_mcp = mcp_feed(c.rig.addr, &c.alice.token, &query).await;
    let mut bob_mcp = mcp_feed(c.rig.addr, &c.bob.token, &query).await;

    c.dm_say(&c.alice, &dm, "to-bob").await;
    c.dm_say(&c.bob, &dm, "to-alice").await;

    for (who, feed) in [
        ("alice ws", &mut alice_ws),
        ("bob ws", &mut bob_ws),
        ("alice mcp", &mut alice_mcp),
        ("bob mcp", &mut bob_mcp),
    ] {
        let saw = feed.until_body("to-alice").await;
        assert!(bodies(&saw).contains(&"to-bob"), "{who} saw {saw:?}");
    }
    c.rig.server.abort();
}

#[tokio::test]
async fn a_member_outside_a_private_channel_made_after_it_subscribed_gets_none_of_its_events() {
    let c = cast().await;
    let mut carol = ws_feed(c.rig.addr, &c.carol.token, c.everything(), 0).await;
    let mut alice = ws_feed(c.rig.addr, &c.alice.token, c.everything(), 0).await;

    let late = c.channel("late", true).await;
    c.add_to_channel(&late, &c.alice).await;
    let thread = c.private_activity(&late, &c.alice).await;
    let end = c.sentinel("end-late").await;

    let carol_saw = carol.until_body(&end).await;
    assert!(
        !mentions_id(&carol_saw, &late) && !mentions_id(&carol_saw, &thread),
        "an outsider receives events of a private channel made after it subscribed: {carol_saw:?}"
    );
    let alice_saw = alice.until_body(&end).await;
    let alice_kinds = kinds(&alice_saw);
    assert!(
        bodies(&alice_saw).contains(&"private-words")
            && alice_kinds.contains(&"reaction_added")
            && alice_kinds.contains(&"vote_cast")
            && alice_kinds.contains(&"mention_recorded"),
        "a member of the new channel receives its events: {alice_kinds:?}"
    );
    c.rig.server.abort();
}

#[tokio::test]
async fn a_reaction_a_vote_and_a_mention_on_a_private_message_do_not_reach_a_non_member() {
    let c = cast().await;
    let private = c.channel("inner", true).await;
    c.add_to_channel(&private, &c.alice).await;
    let mut carol = ws_feed(c.rig.addr, &c.carol.token, c.everything(), 0).await;
    let granting = c.granting(&[&private, &c.public_channel]);
    let mut alice = ws_feed(c.rig.addr, &c.alice.token, granting, 0).await;

    let thread = c.private_activity(&private, &c.alice).await;
    let end = c.sentinel("end-activity").await;

    let carol_saw = carol.until_body(&end).await;
    assert!(
        !mentions_id(&carol_saw, &thread) && !mentions_id(&carol_saw, &private),
        "an outsider receives events of a private channel: {carol_saw:?}"
    );
    let alice_saw = alice.until_body(&end).await;
    let alice_kinds = kinds(&alice_saw);
    assert!(
        alice_kinds.contains(&"reaction_added")
            && alice_kinds.contains(&"vote_cast")
            && alice_kinds.contains(&"mention_recorded"),
        "a member receives them: {alice_kinds:?}"
    );
    c.rig.server.abort();
}

#[tokio::test]
async fn a_member_removed_from_a_private_channel_mid_stream_stops_receiving_it() {
    let c = cast().await;
    let private = c.channel("leaving", true).await;
    c.add_to_channel(&private, &c.alice).await;
    let thread = c.thread(&private).await;
    let granting = c.granting(&[&private, &c.public_channel]);
    let mut alice = ws_feed(c.rig.addr, &c.alice.token, granting, 0).await;

    c.say(&c.admin, &thread, "before-removal").await;
    alice.until_body("before-removal").await;

    let removed = c
        .admin
        .delete(format!(
            "{}/channels/{private}/members/{}",
            c.base, c.alice.member_id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), 204);
    // The stream reuses an answer for a few seconds, so a removal is honored
    // within that, not instantly.
    tokio::time::sleep(PAST_VISIBILITY_LIFETIME).await;
    c.say(&c.admin, &thread, "after-removal").await;
    let end = c.sentinel("end-removal").await;

    let saw = alice.until_body(&end).await;
    assert!(
        !bodies(&saw).contains(&"after-removal"),
        "a removed member still receives the channel: {saw:?}"
    );
    c.rig.server.abort();
}

#[tokio::test]
async fn a_public_channel_event_reaches_everyone_and_an_admin_stream_is_unchanged() {
    let c = cast().await;
    let private = c.channel("admins", true).await;
    let private_thread = c.thread(&private).await;
    let mut carol = ws_feed(c.rig.addr, &c.carol.token, c.everything(), 0).await;
    let mut admin = ws_feed(
        c.rig.addr,
        &c.rig.tenant.token,
        c.granting(&[&private, &c.public_channel]),
        0,
    )
    .await;

    c.say(&c.admin, &private_thread, "admins-only").await;
    c.say(&c.admin, &c.public_thread, "for-everyone").await;
    let end = c.sentinel("end-public").await;

    let carol_saw = carol.until_body(&end).await;
    assert!(bodies(&carol_saw).contains(&"for-everyone"));
    assert!(!bodies(&carol_saw).contains(&"admins-only"));
    let admin_saw = admin.until_body(&end).await;
    let admin_bodies = bodies(&admin_saw);
    assert!(admin_bodies.contains(&"for-everyone") && admin_bodies.contains(&"admins-only"));
    c.rig.server.abort();
}
