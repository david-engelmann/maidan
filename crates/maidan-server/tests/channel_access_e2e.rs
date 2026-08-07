//! Cluster 160: per-channel authorization enforcement over REST.
//!
//! Runs with auth ENABLED (real tokens) so `ensure_channel_access` is
//! exercised: a non-member is denied read/write in a private channel; the
//! creator (auto-added) and explicit members are allowed; public channels and
//! DM threads are unaffected.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    ChannelId, ChannelMemberRole, MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Ctx {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

impl Ctx {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }
}

async fn mint(store: &dyn Store, ws: maidan_types::WorkspaceId, member: MemberId) -> String {
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
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
                capability::THREAD_TRANSITION.into(),
                capability::CHANNEL_ADMIN.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn spawn() -> Ctx {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
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
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ctx {
        addr,
        server,
        client: reqwest::Client::new(),
        store,
        _dir: dir,
    }
}

#[tokio::test]
async fn private_channel_denies_non_members_over_rest() {
    let ctx = spawn().await;
    let base = ctx.base();

    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let mk_member = |handle: &'static str| {
        let store = ctx.store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
        }
    };
    let alice = mk_member("alice").await;
    let bob = mk_member("bob").await;
    let alice_tok = mint(ctx.store.as_ref(), ws.id, alice.id).await;
    let bob_tok = mint(ctx.store.as_ref(), ws.id, bob.id).await;

    let auth = |t: &str| format!("Bearer {t}");

    // Alice creates a PRIVATE channel — she is auto-added as admin.
    let ch: Value = ctx
        .client
        .post(format!("{base}/workspaces/{}/channels", ws.id.0))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"name": "secret", "private": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();

    // Alice creates a thread + posts a message (she's a member).
    let th: Value = ctx
        .client
        .post(format!("{base}/channels/{cid}/threads"))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"title": "plans"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();
    let post_alice = ctx
        .client
        .post(format!("{base}/threads/{tid}/messages"))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"author_id": alice.id.0, "body": "top secret"}))
        .send()
        .await
        .unwrap();
    assert_eq!(post_alice.status(), StatusCode::CREATED, "creator can post");
    let list_alice = ctx
        .client
        .get(format!("{base}/threads/{tid}/messages"))
        .header("Authorization", auth(&alice_tok))
        .send()
        .await
        .unwrap();
    assert_eq!(list_alice.status(), StatusCode::OK, "creator can read");

    // Bob is NOT a member → denied read + write + thread/channel access.
    let denied = |resp: reqwest::Response, what: &str| {
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "bob denied: {what}");
    };
    denied(
        ctx.client
            .post(format!("{base}/threads/{tid}/messages"))
            .header("Authorization", auth(&bob_tok))
            .json(&json!({"author_id": bob.id.0, "body": "sneak"}))
            .send()
            .await
            .unwrap(),
        "post",
    );
    denied(
        ctx.client
            .get(format!("{base}/threads/{tid}/messages"))
            .header("Authorization", auth(&bob_tok))
            .send()
            .await
            .unwrap(),
        "list messages",
    );
    denied(
        ctx.client
            .get(format!("{base}/threads/{tid}"))
            .header("Authorization", auth(&bob_tok))
            .send()
            .await
            .unwrap(),
        "get thread",
    );
    denied(
        ctx.client
            .get(format!("{base}/channels/{cid}"))
            .header("Authorization", auth(&bob_tok))
            .send()
            .await
            .unwrap(),
        "get channel",
    );
    // Cluster 165: references into the private thread are gated too.
    denied(
        ctx.client
            .get(format!("{base}/references?src_kind=thread&src_id={tid}"))
            .header("Authorization", auth(&bob_tok))
            .send()
            .await
            .unwrap(),
        "list references",
    );

    // Add Bob as an explicit member → now allowed.
    ctx.store
        .add_channel_member(
            ChannelId(cid.parse().unwrap()),
            bob.id,
            ChannelMemberRole::Member,
        )
        .await
        .unwrap();
    let list_bob = ctx
        .client
        .get(format!("{base}/threads/{tid}/messages"))
        .header("Authorization", auth(&bob_tok))
        .send()
        .await
        .unwrap();
    assert_eq!(list_bob.status(), StatusCode::OK, "member bob can now read");

    ctx.server.abort();
}

#[tokio::test]
async fn public_channel_and_dm_are_unaffected() {
    let ctx = spawn().await;
    let base = ctx.base();

    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let alice = ctx
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let bob = ctx
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let alice_tok = mint(ctx.store.as_ref(), ws.id, alice.id).await;
    let bob_tok = mint(ctx.store.as_ref(), ws.id, bob.id).await;
    let auth = |t: &str| format!("Bearer {t}");

    // PUBLIC channel: any workspace member can read + write (no membership).
    let ch: Value = ctx
        .client
        .post(format!("{base}/workspaces/{}/channels", ws.id.0))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"name": "general", "private": false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();
    let th: Value = ctx
        .client
        .post(format!("{base}/channels/{cid}/threads"))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"title": "hi"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();
    // Bob (not "in" the public channel — nobody is) can post + read.
    let bob_post = ctx
        .client
        .post(format!("{base}/threads/{tid}/messages"))
        .header("Authorization", auth(&bob_tok))
        .json(&json!({"author_id": bob.id.0, "body": "hello public"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        bob_post.status(),
        StatusCode::CREATED,
        "public channel open to workspace"
    );

    // DM: a participant reads DM messages via the generic thread route — the
    // __dm__ channel is exempt, so enforcement doesn't break DMs.
    let dm = ctx
        .store
        .open_dm_conversation(ws.id, alice.id, bob.id)
        .await
        .unwrap();
    ctx.store
        .post_message(maidan_types::NewMessage {
            thread_id: dm.thread_id,
            author_id: alice.id,
            body: "dm hi".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();
    let dm_read = ctx
        .client
        .get(format!("{base}/threads/{}/messages", dm.thread_id.0))
        .header("Authorization", auth(&bob_tok))
        .send()
        .await
        .unwrap();
    assert_eq!(
        dm_read.status(),
        StatusCode::OK,
        "DM thread readable (channel exemption)"
    );

    ctx.server.abort();
}

#[tokio::test]
async fn channel_admin_api_manages_membership_end_to_end() {
    let ctx = spawn().await;
    let base = ctx.base();
    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let mk = |handle: &'static str| {
        let store = ctx.store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
        }
    };
    let alice = mk("alice").await;
    let bob = mk("bob").await;
    let alice_tok = mint(ctx.store.as_ref(), ws.id, alice.id).await; // has channel:admin
    let bob_tok = mint(ctx.store.as_ref(), ws.id, bob.id).await;
    let auth = |t: &str| format!("Bearer {t}");

    // Alice creates a private channel + thread.
    let ch: Value = ctx
        .client
        .post(format!("{base}/workspaces/{}/channels", ws.id.0))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"name": "secret", "private": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();
    let th: Value = ctx
        .client
        .post(format!("{base}/channels/{cid}/threads"))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"title": "t"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();

    // Bob denied before being added.
    assert_eq!(
        ctx.client
            .get(format!("{base}/threads/{tid}/messages"))
            .header("Authorization", auth(&bob_tok))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );

    // Add Bob via the management API (alice has channel:admin).
    let add = ctx
        .client
        .post(format!("{base}/channels/{cid}/members"))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"member_id": bob.id.0}))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::CREATED);

    // Now Bob can read.
    assert_eq!(
        ctx.client
            .get(format!("{base}/threads/{tid}/messages"))
            .header("Authorization", auth(&bob_tok))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    // List shows Alice (auto-added admin) + Bob.
    let members: Value = ctx
        .client
        .get(format!("{base}/channels/{cid}/members"))
        .header("Authorization", auth(&alice_tok))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(members.as_array().unwrap().len(), 2);

    // Remove Bob → denied again.
    let del = ctx
        .client
        .delete(format!("{base}/channels/{cid}/members/{}", bob.id.0))
        .header("Authorization", auth(&alice_tok))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        ctx.client
            .get(format!("{base}/threads/{tid}/messages"))
            .header("Authorization", auth(&bob_tok))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );

    ctx.server.abort();
}
