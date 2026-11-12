//! The event log reads back only what the reader may see. `GET
//! /workspaces/:wid/events` and `/events/catch-up` returned every event in the
//! workspace to any `workspace:read` token, bodies included: a private channel's
//! messages to non-members, and every DM to every member. The live streams
//! already filtered these; the backfill now matches them, and the unfiltered
//! chain (catch-up) is a workspace admin's read.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelMemberRole, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread,
    NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

struct Ctx {
    addr: SocketAddr,
    _server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}
impl Ctx {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }
}

async fn spawn() -> Ctx {
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ctx {
        addr,
        _server: server,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        store,
        _dir: dir,
    }
}

async fn mk_member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
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

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    format!("Bearer {}", secret.as_str())
}

async fn post(ctx: &Ctx, token: &str, path: String, body: Value) {
    let resp = ctx
        .client
        .post(format!("{}{path}", ctx.base()))
        .header("Authorization", token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{path}: {}", resp.status());
}

/// Every event body a reader gets back from the backfill routes.
async fn what_reader_sees(ctx: &Ctx, token: &str, ws: WorkspaceId) -> String {
    let mut seen = String::new();
    for path in [
        format!("/workspaces/{}/events?limit=500", ws.0),
        format!("/ui/api/workspaces/{}/events?limit=500", ws.0),
        format!("/workspaces/{}/events?order=desc&limit=500", ws.0),
        format!("/ui/api/workspaces/{}/events?order=desc&limit=500", ws.0),
    ] {
        let resp = ctx
            .client
            .get(format!("{}{path}", ctx.base()))
            .header("Authorization", token)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        seen.push_str(&resp.text().await.unwrap());
    }
    seen
}

#[tokio::test]
async fn the_event_log_hides_what_the_reader_cannot_see() {
    let ctx = spawn().await;
    let store = ctx.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap()
        .id;
    let alice = mk_member(store, ws, "alice").await;
    let bob = mk_member(store, ws, "bob").await;
    let carol = mk_member(store, ws, "carol").await;
    let (alice_t, bob_t, carol_t) = (
        mint(store, ws, alice).await,
        mint(store, ws, bob).await,
        mint(store, ws, carol).await,
    );

    let secret = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "secret".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    store
        .add_channel_member(secret.id, alice, ChannelMemberRole::Member)
        .await
        .unwrap();
    let hidden = store
        .create_thread(NewThread {
            channel_id: secret.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    post(
        &ctx,
        &alice_t,
        format!("/threads/{}/messages", hidden.id.0),
        json!({"body": "private-channel-words"}),
    )
    .await;

    let dm = store.open_dm_conversation(ws, alice, bob).await.unwrap();
    post(
        &ctx,
        &alice_t,
        format!("/dm/{}/messages", dm.id.0),
        json!({"body": "dm-words"}),
    )
    .await;

    let general = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let open = store
        .create_thread(NewThread {
            channel_id: general.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    post(
        &ctx,
        &carol_t,
        format!("/threads/{}/messages", open.id.0),
        json!({"body": "public-words"}),
    )
    .await;

    // A message posted, edited, then withdrawn by its author.
    let resp = ctx
        .client
        .post(format!("{}/threads/{}/messages", ctx.base(), open.id.0))
        .header("Authorization", &alice_t)
        .json(&json!({"body": "withdrawn-words"}))
        .send()
        .await
        .unwrap();
    let withdrawn: Value = resp.json().await.unwrap();
    let wid = withdrawn["id"].as_str().unwrap().to_string();
    for (method, body) in [
        (
            reqwest::Method::PATCH,
            Some(json!({"body": "withdrawn-words-edited"})),
        ),
        (reqwest::Method::DELETE, None),
    ] {
        let mut req = ctx
            .client
            .request(method, format!("{}/messages/{wid}", ctx.base()))
            .header("Authorization", &alice_t);
        if let Some(body) = body {
            req = req.json(&body);
        }
        assert!(req.send().await.unwrap().status().is_success());
    }

    let carol_sees = what_reader_sees(&ctx, &carol_t, ws).await;
    assert!(
        !carol_sees.contains("withdrawn-words"),
        "a withdrawn message's words came back through the log"
    );
    assert!(
        carol_sees.contains(&wid),
        "the withdrawn message keeps its place in the log"
    );
    assert!(
        carol_sees.contains("public-words"),
        "the public channel is still readable"
    );
    assert!(
        !carol_sees.contains("private-channel-words"),
        "a non-member read a private channel through the log"
    );
    assert!(
        !carol_sees.contains("dm-words"),
        "a non-participant read a DM through the log"
    );

    let bob_sees = what_reader_sees(&ctx, &bob_t, ws).await;
    assert!(
        bob_sees.contains("dm-words"),
        "a DM participant still reads the DM"
    );
    assert!(!bob_sees.contains("private-channel-words"));

    // The whole log, one verifiable chain, is a workspace admin's read.
    let catch_up = ctx
        .client
        .get(format!(
            "{}/workspaces/{}/events/catch-up?after_lsn=0",
            ctx.base(),
            ws.0
        ))
        .header("Authorization", &carol_t)
        .send()
        .await
        .unwrap();
    assert_eq!(catch_up.status(), StatusCode::FORBIDDEN);

    // The whole chain, words included, is the admin tier's.
    let admin = mk_member(store, ws, "admin").await;
    let admin_t = {
        let secret = TokenSecret::generate();
        store
            .create_api_token(NewApiToken {
                workspace_id: ws,
                member_id: admin,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: vec![
                    capability::WORKSPACE_READ.into(),
                    capability::TOKEN_ADMIN.into(),
                ],
                expires_at: None,
            })
            .await
            .unwrap();
        format!("Bearer {}", secret.as_str())
    };
    let whole = ctx
        .client
        .get(format!(
            "{}/workspaces/{}/events/catch-up?after_lsn=0&limit=500",
            ctx.base(),
            ws.0
        ))
        .header("Authorization", &admin_t)
        .send()
        .await
        .unwrap();
    assert_eq!(whole.status(), StatusCode::OK);
    // The chain verified (200) over sealed payloads. Live words open with the
    // keys the page carries; the withdrawn message's key was shredded, so its
    // words are ciphertext nobody can open, admins included.
    let whole = whole.text().await.unwrap();
    assert!(!whole.contains("withdrawn-words"));
    let page: Value = serde_json::from_str(&whole).unwrap();
    let opened: String = page["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            let event: maidan_types::StoredEvent = serde_json::from_value(event.clone()).unwrap();
            event.opened_payload().unwrap().to_string()
        })
        .collect();
    assert!(opened.contains("public-words"));
    assert!(!opened.contains("withdrawn-words"));

    let alice_sees = what_reader_sees(&ctx, &alice_t, ws).await;
    assert!(
        !alice_sees.contains("withdrawn-words"),
        "withdrawn is withdrawn for its author too"
    );
    for words in ["public-words", "private-channel-words", "dm-words"] {
        assert!(alice_sees.contains(words), "alice may see {words}");
    }
}

async fn event_ids(ctx: &Ctx, token: &str, path: String) -> Vec<i64> {
    let resp = ctx
        .client
        .get(format!("{}{path}", ctx.base()))
        .header("Authorization", token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{path}");
    let rows: Vec<Value> = resp.json().await.unwrap();
    rows.iter().map(|row| row["id"].as_i64().unwrap()).collect()
}

async fn status_of(ctx: &Ctx, token: &str, path: String) -> StatusCode {
    ctx.client
        .get(format!("{}{path}", ctx.base()))
        .header("Authorization", token)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn newest_first_pages_back_through_the_log_the_reader_may_see() {
    let ctx = spawn().await;
    let store = ctx.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap()
        .id;
    let alice = mk_member(store, ws, "alice").await;
    let carol = mk_member(store, ws, "carol").await;
    let (alice_t, carol_t) = (mint(store, ws, alice).await, mint(store, ws, carol).await);
    let secret = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "secret".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    store
        .add_channel_member(secret.id, alice, ChannelMemberRole::Member)
        .await
        .unwrap();
    let general = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let mut threads = Vec::new();
    for channel in [general.id, secret.id] {
        threads.push(
            store
                .create_thread(NewThread {
                    channel_id: channel,
                    parent_thread_id: None,
                    title: None,
                    description: None,
                    created_by: None,
                })
                .await
                .unwrap()
                .id,
        );
    }
    // Hidden rows sit between and at the head of carol's visible ones, so a
    // page that stopped at a hidden row would come back short.
    for n in 0..7 {
        post(
            &ctx,
            &alice_t,
            format!("/threads/{}/messages", threads[0].0),
            json!({"body": format!("public-{n}")}),
        )
        .await;
        post(
            &ctx,
            &alice_t,
            format!("/threads/{}/messages", threads[1].0),
            json!({"body": format!("private-{n}")}),
        )
        .await;
    }

    let forward = event_ids(
        &ctx,
        &carol_t,
        format!("/workspaces/{}/events?limit=500", ws.0),
    )
    .await;
    assert_eq!(forward.len(), 7, "carol sees the public posts only");
    let newest_first: Vec<i64> = forward.iter().rev().copied().collect();

    let head = event_ids(
        &ctx,
        &carol_t,
        format!("/workspaces/{}/events?order=desc&limit=3", ws.0),
    )
    .await;
    assert_eq!(head, newest_first[..3], "one read returns the newest page");

    let mut paged = Vec::new();
    let mut before: Option<i64> = None;
    loop {
        let path = match before {
            None => format!("/ui/api/workspaces/{}/events?order=desc&limit=2", ws.0),
            Some(id) => format!(
                "/ui/api/workspaces/{}/events?order=desc&limit=2&before_id={id}",
                ws.0
            ),
        };
        let page = event_ids(&ctx, &carol_t, path).await;
        if page.is_empty() {
            break;
        }
        assert!(
            page.len() == 2 || paged.len() + page.len() == 7,
            "no short page mid-log"
        );
        before = page.last().copied();
        paged.extend(page);
    }
    assert_eq!(
        paged, newest_first,
        "before_id pages back with no gap or repeat"
    );

    // The member of the private channel sees both, still newest first.
    let alice_all = event_ids(
        &ctx,
        &alice_t,
        format!("/workspaces/{}/events?order=desc&limit=500", ws.0),
    )
    .await;
    assert_eq!(alice_all.len(), 14);
    assert!(alice_all.windows(2).all(|w| w[0] > w[1]));

    // The filters apply the same way newest first.
    let one_thread = event_ids(
        &ctx,
        &alice_t,
        format!(
            "/workspaces/{}/events?order=desc&limit=500&thread_id={}&types=message_posted",
            ws.0, threads[0].0
        ),
    )
    .await;
    assert_eq!(one_thread, newest_first);
}

#[tokio::test]
async fn newest_first_refuses_a_forward_cursor_and_another_tenant() {
    let ctx = spawn().await;
    let store = ctx.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap()
        .id;
    let other = store
        .create_workspace(NewWorkspace {
            name: "globex".into(),
        })
        .await
        .unwrap()
        .id;
    let carol = mk_member(store, ws, "carol").await;
    let carol_t = mint(store, ws, carol).await;
    let outsider = mk_member(store, other, "outsider").await;
    let outsider_t = mint(store, other, outsider).await;

    let base = format!("/workspaces/{}/events", ws.0);
    for query in [
        "order=desc&after_id=1",
        "order=desc&consumer_id=reader",
        "order=desc&before_id=-1",
        "before_id=5",
        "order=sideways",
    ] {
        assert_eq!(
            status_of(&ctx, &carol_t, format!("{base}?{query}")).await,
            StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    assert_eq!(
        status_of(&ctx, &carol_t, format!("{base}?order=desc")).await,
        StatusCode::OK
    );
    for path in [
        format!("{base}?order=desc"),
        format!("/ui/api{base}?order=desc&before_id=100"),
    ] {
        assert_eq!(
            status_of(&ctx, &outsider_t, path.clone()).await,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
}
