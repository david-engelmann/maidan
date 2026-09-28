//! Crypto-shredding over HTTP (SQLite, auth on). A withdrawn message's words
//! are gone from every surface a reader, an admin or a federation peer can
//! reach, while the hash chain over the log still verifies. A peer that
//! replicated the words shreds its own copy on the origin's tombstone, and a
//! peer that replicates after the withdrawal receives only ciphertext. A shared
//! artifact survives until the last workspace erases it.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_a2a::{FederatedEventBatch, FederationEnvelope};
use maidan_artifacts::{ArtifactStore, LocalFsStore, Sha256};
use maidan_auth::{capability, hash_secret, ExportSigningKey, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ContentKeyring, Event, EventKind, MemberId, MemberKind, NewApiToken, NewChannel, NewMember,
    NewThread, NewWorkspace, PeerId, StoredEvent, ThreadId, WorkspaceId,
};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const SECRET: &str = "SECRET-WORDS-7d1f";
const SECRET_META: &str = "SECRET-META-7d1f";
const KEPT: &str = "KEPT-WORDS-7d1f";

struct Ctx {
    addr: SocketAddr,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    artifacts: Arc<LocalFsStore>,
    _server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Ctx {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn call(
        &self,
        method: Method,
        token: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, String) {
        let mut req = self
            .client
            .request(method, self.url(path))
            .bearer_auth(token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.unwrap();
        (resp.status(), resp.text().await.unwrap())
    }

    async fn get_ok(&self, token: &str, path: &str) -> String {
        let (status, text) = self.call(Method::GET, token, path, None).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {text}");
        text
    }

    async fn mcp(&self, token: &str, tool: &str, arguments: Value) -> String {
        let (status, text) = self
            .call(
                Method::POST,
                token,
                "/mcp",
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {"name": tool, "arguments": arguments},
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{tool}: {text}");
        let reply: Value = serde_json::from_str(&text).unwrap();
        assert!(reply.get("error").is_none(), "{tool}: {text}");
        text
    }
}

async fn spawn(kek: u8) -> Ctx {
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let keys = Arc::new(ContentKeyring::new([kek; 32], Vec::new()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()).with_content_keys(keys));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let mut state = AppState::new(
        store.clone(),
        artifacts.clone(),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth enabled
        true,
        FederationRuntime::new(true, Some(Arc::new([0x42; 32]))),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    state.attach_export_signing(ExportSigningKey::from_seed([0x11; 32]));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ctx {
        addr,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        artifacts,
        _server: server,
        _dir: dir,
    }
}

struct Room {
    ws: WorkspaceId,
    thread: ThreadId,
    alice: String,
    admin: String,
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
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

async fn room(ctx: &Ctx, name: &str) -> Room {
    let store = ctx.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let member_caps = [
        capability::WORKSPACE_READ,
        capability::WORKSPACE_WRITE,
        capability::MESSAGE_POST,
        capability::SEARCH_QUERY,
    ];
    let alice_id = member(store, ws, "alice").await;
    let alice = mint(store, ws, alice_id, &member_caps).await;
    let admin_id = member(store, ws, "admin").await;
    let admin = mint(
        store,
        ws,
        admin_id,
        &[
            &member_caps[..],
            &[
                capability::TOKEN_ADMIN,
                capability::FEDERATION_ADMIN,
                capability::ARTIFACT_UPLOAD,
            ],
        ]
        .concat(),
    )
    .await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
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
            title: None,
        })
        .await
        .unwrap()
        .id;
    Room {
        ws,
        thread,
        alice,
        admin,
    }
}

async fn say(ctx: &Ctx, room: &Room, body: &str, metadata: Value) -> String {
    let (status, text) = ctx
        .call(
            Method::POST,
            &room.alice,
            &format!("/threads/{}/messages", room.thread.0),
            Some(json!({"body": body, "metadata": metadata})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    serde_json::from_str::<Value>(&text).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn withdraw(ctx: &Ctx, room: &Room, id: &str) {
    let (status, text) = ctx
        .call(
            Method::DELETE,
            &room.alice,
            &format!("/messages/{id}"),
            None,
        )
        .await;
    assert!(status.is_success(), "withdraw: {status} {text}");
}

fn assert_no_secret(surface: &str, text: &str) {
    assert!(
        !text.contains(SECRET),
        "{surface} still shows the withdrawn words"
    );
    assert!(
        !text.contains(SECRET_META),
        "{surface} still shows the withdrawn metadata"
    );
}

#[tokio::test]
async fn withdrawn_words_are_unreadable_on_every_surface() {
    let ctx = spawn(1).await;
    let room = room(&ctx, "shred").await;
    let ws = room.ws.0;
    let id = say(&ctx, &room, SECRET, json!({"hint": SECRET_META})).await;
    say(&ctx, &room, KEPT, json!({})).await;

    let events = format!("/workspaces/{ws}/events?after_id=0&limit=500");
    let search = format!("/workspaces/{ws}/search?q={SECRET}");
    // Before: a member reads the words back, and search finds them.
    assert!(ctx.get_ok(&room.alice, &events).await.contains(SECRET));
    assert!(ctx.get_ok(&room.alice, &search).await.contains(SECRET));

    withdraw(&ctx, &room, &id).await;

    let member_events = ctx.get_ok(&room.alice, &events).await;
    assert_no_secret("a member's event log", &member_events);
    assert!(member_events.contains(KEPT), "other words stay readable");
    assert!(member_events.contains(&id), "the message keeps its place");
    for (surface, path) in [
        ("an admin's event log", events.clone()),
        (
            "catch-up",
            format!("/workspaces/{ws}/events/catch-up?after_lsn=0&limit=500"),
        ),
        ("export", format!("/workspaces/{ws}/export")),
        ("snapshot", format!("/workspaces/{ws}/snapshot")),
        ("search", search.clone()),
        ("the thread", format!("/threads/{}/messages", room.thread.0)),
    ] {
        assert_no_secret(surface, &ctx.get_ok(&room.admin, &path).await);
    }
    for (tool, args) in [
        (
            "catch_up_events",
            json!({"workspace_id": ws, "after_lsn": 0}),
        ),
        ("get_log_snapshot", json!({"workspace_id": ws})),
        (
            "search_messages",
            json!({"workspace_id": ws, "query": SECRET}),
        ),
    ] {
        assert_no_secret(tool, &ctx.mcp(&room.admin, tool, args).await);
    }

    // The log was never rewritten, so its chain still verifies.
    let verify = ctx
        .get_ok(&room.admin, &format!("/workspaces/{ws}/events/verify"))
        .await;
    assert_eq!(serde_json::from_str::<Value>(&verify).unwrap()["ok"], true);
    let catch_up: Value = serde_json::from_str(
        &ctx.get_ok(
            &room.admin,
            &format!("/workspaces/{ws}/events/catch-up?after_lsn=0&limit=500"),
        )
        .await,
    )
    .unwrap();
    let events: Vec<StoredEvent> = serde_json::from_value(catch_up["events"].clone()).unwrap();
    let links: Vec<_> = events.iter().map(StoredEvent::link).collect();
    let payloads: Vec<_> = events.iter().map(|e| e.payload.clone()).collect();
    assert!(maidan_types::verify_chain(&links, &payloads).ok);
    // Live words still open with the keys the page carries.
    let opened: String = events
        .iter()
        .map(|e| e.opened_payload().unwrap().to_string())
        .collect();
    assert!(opened.contains(KEPT));
    assert_no_secret("opened catch-up", &opened);
}

/// Peer `name` registered on `host`, for `host`'s workspace `ws`. Returns the
/// peer id and the bearer the peer presents.
async fn peer(host: &Ctx, admin: &str, ws: WorkspaceId, name: &str) -> (PeerId, String) {
    let (status, text) = host
        .call(
            Method::POST,
            admin,
            &format!("/workspaces/{}/peers", ws.0),
            Some(json!({"name": name, "base_url": "https://peer.example"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    (
        PeerId(body["peer"]["id"].as_str().unwrap().parse().unwrap()),
        body["secret"].as_str().unwrap().to_string(),
    )
}

/// What a peer pulls from `origin` after `after_id`.
async fn pull(origin: &Ctx, bearer: &str, ws: WorkspaceId, after_id: i64) -> Vec<StoredEvent> {
    let text = origin
        .get_ok(
            bearer,
            &format!("/workspaces/{}/events?after_id={after_id}&limit=500", ws.0),
        )
        .await;
    serde_json::from_str(&text).unwrap()
}

/// Push pulled events into `receiver` as the origin peer `origin_peer`.
async fn push(receiver: &Ctx, bearer: &str, origin_peer: PeerId, events: Vec<StoredEvent>) {
    let batch = FederatedEventBatch {
        events: events
            .into_iter()
            .map(|event| FederationEnvelope {
                origin_peer_id: origin_peer,
                remote_event_id: event.id,
                event,
            })
            .collect(),
    };
    let resp = receiver
        .client
        .post(receiver.url("/a2a/v1/events"))
        .bearer_auth(bearer)
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "{}",
        resp.text().await.unwrap()
    );
}

async fn words_in(receiver: &Ctx, ws: WorkspaceId) -> String {
    receiver
        .store
        .list_events_after(ws, 0, 500)
        .await
        .unwrap()
        .iter()
        .map(|e| format!("{}{}", e.payload, e.opened_payload().unwrap()))
        .collect()
}

#[tokio::test]
async fn peers_get_ciphertext_only_for_shredded_words() {
    let origin = spawn(1).await;
    let room = room(&origin, "origin").await;
    let id = say(&origin, &room, SECRET, json!({})).await;
    let (_, pull_bearer) = peer(&origin, &room.admin, room.ws, "replica").await;

    // A peer pulls sealed payloads with their live keys.
    let before = pull(&origin, &pull_bearer, room.ws, 0).await;
    let wire = serde_json::to_string(
        &before
            .iter()
            .map(|e| maidan_types::KeyedEvent(e.clone()))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(!wire.contains(SECRET), "words travel sealed");
    let posted = before
        .iter()
        .find(|e| e.kind == EventKind::MessagePosted)
        .unwrap();
    assert!(posted.content_key.is_some());
    assert!(posted
        .opened_payload()
        .unwrap()
        .to_string()
        .contains(SECRET));

    // An early replica stores the words under its own key.
    let early = spawn(2).await;
    let early_room = self::room(&early, "early").await;
    let (origin_on_early, early_bearer) =
        peer(&early, &early_room.admin, early_room.ws, "origin").await;
    let head = before.last().unwrap().id;
    push(&early, &early_bearer, origin_on_early, before).await;
    assert!(words_in(&early, early_room.ws).await.contains(SECRET));

    withdraw(&origin, &room, &id).await;

    // After the withdrawal the origin serves the event with no key.
    let after = pull(&origin, &pull_bearer, room.ws, 0).await;
    let posted = after
        .iter()
        .find(|e| e.kind == EventKind::MessagePosted)
        .unwrap();
    assert!(posted.content_key.is_none());
    assert!(posted.is_shredded());

    // The early replica shreds its copy on the origin's tombstone.
    let tail: Vec<_> = after.iter().filter(|e| e.id > head).cloned().collect();
    assert!(tail.iter().any(|e| e.kind == EventKind::MessageTombstoned));
    push(&early, &early_bearer, origin_on_early, tail).await;
    assert_no_secret("the early replica", &words_in(&early, early_room.ws).await);
    assert!(
        early
            .store
            .verify_event_chain(early_room.ws)
            .await
            .unwrap()
            .ok
    );

    // A late replica only ever receives ciphertext.
    let late = spawn(3).await;
    let late_room = self::room(&late, "late").await;
    let (origin_on_late, late_bearer) = peer(&late, &late_room.admin, late_room.ws, "origin").await;
    push(&late, &late_bearer, origin_on_late, after).await;
    let late_words = words_in(&late, late_room.ws).await;
    assert_no_secret("the late replica", &late_words);
    let ingested = late
        .store
        .list_events_after(late_room.ws, 0, 500)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.kind == EventKind::MessagePosted)
        .unwrap();
    assert!(ingested.is_shredded());
    assert!(matches!(
        ingested.opened_event().unwrap(),
        Event::MessagePosted {
            sealed: Some(_),
            ..
        }
    ));
}

#[tokio::test]
async fn a_shared_artifact_is_deleted_with_its_last_reference() {
    let ctx = spawn(1).await;
    let a = room(&ctx, "a").await;
    let b = room(&ctx, "b").await;
    let bytes = b"shared artifact bytes".to_vec();
    let mut sha = String::new();
    for room in [&a, &b] {
        let resp = ctx
            .client
            .post(ctx.url("/artifacts?kind=attachment"))
            .bearer_auth(&room.admin)
            .body(bytes.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        sha = resp.json::<Value>().await.unwrap()["sha256"]
            .as_str()
            .unwrap()
            .to_string();
    }
    let path = format!("/artifacts/{sha}");
    let blob = Sha256::from_hex(&sha).unwrap();

    // A member cannot erase; an admin erases their workspace's copy only.
    let (status, _) = ctx.call(Method::DELETE, &a.alice, &path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, text) = ctx.call(Method::DELETE, &a.admin, &path, None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["last_reference"],
        false
    );
    let (status, _) = ctx.call(Method::GET, &a.admin, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        ctx.get_ok(&b.admin, &path).await,
        String::from_utf8(bytes).unwrap()
    );
    assert!(ctx.artifacts.get(&blob).await.is_ok());
    let (status, _) = ctx.call(Method::DELETE, &a.admin, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The last reference takes the bytes.
    let (status, text) = ctx.call(Method::DELETE, &b.admin, &path, None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["last_reference"],
        true
    );
    let (status, _) = ctx.call(Method::GET, &b.admin, &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(ctx.artifacts.get(&blob).await.is_err());
    let audit = ctx.store.list_audit_for_workspace(b.ws, 20).await.unwrap();
    assert!(audit.iter().any(|e| e.action == "artifact.erase"));
}
