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
use maidan_store::{prelude::*, run_sqlite_migrations};
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

/// Cluster 179: the A2A JSON-RPC ingress (`POST /a2a/v1/rpc`) enforces per-channel
/// access. A non-member holding `message:post` is denied posting into a private
/// channel's thread — the surface the 160–165 RBAC arc had missed.
#[tokio::test]
async fn a2a_ingress_denies_non_members_in_private_channels() {
    let ctx = spawn().await;
    let base = ctx.base();

    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "a2a-acme".into(),
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
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
        }
    };
    let alice = mk("alice").await;
    let mallory = mk("mallory").await;
    let alice_tok = mint(ctx.store.as_ref(), ws.id, alice.id).await;
    let mallory_tok = mint(ctx.store.as_ref(), ws.id, mallory.id).await;
    let auth = |t: &str| format!("Bearer {t}");

    // Alice creates a PRIVATE channel (auto-added) + a thread.
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
        .json(&json!({"title": "plans"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();

    let a2a_body = |member: uuid::Uuid| {
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
            "params": {
                "message": {"role": "user", "parts": [{"type": "text", "text": "via a2a"}]},
                "metadata": {"maidan": {"threadId": tid, "authorId": member}}
            }
        })
    };

    // Mallory (not a channel member) is denied — JSON-RPC error, no message posted.
    let denied: Value = ctx
        .client
        .post(format!("{base}/a2a/v1/rpc"))
        .header("Authorization", auth(&mallory_tok))
        .json(&a2a_body(mallory.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        denied.get("error").is_some(),
        "non-member A2A post must be rejected, got {denied}"
    );
    assert!(denied.get("result").is_none());

    // Alice (the channel member) can post via A2A.
    let ok: Value = ctx
        .client
        .post(format!("{base}/a2a/v1/rpc"))
        .header("Authorization", auth(&alice_tok))
        .json(&a2a_body(alice.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        ok.get("result").is_some(),
        "member A2A post should succeed: {ok}"
    );

    ctx.server.abort();
}

/// Cluster 180: a DM thread is NOT readable via the generic `/threads/:id` route
/// by a non-participant (the `__dm__` exemption previously left this open);
/// participants still read it.
#[tokio::test]
async fn dm_thread_not_readable_via_generic_route_by_non_participant() {
    let ctx = spawn().await;
    let base = ctx.base();

    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "dm-acme".into(),
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
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
        }
    };
    let alice = mk("alice").await;
    let bob = mk("bob").await;
    let mallory = mk("mallory").await;
    let alice_tok = mint(ctx.store.as_ref(), ws.id, alice.id).await;
    let bob_tok = mint(ctx.store.as_ref(), ws.id, bob.id).await;
    let mallory_tok = mint(ctx.store.as_ref(), ws.id, mallory.id).await;
    let auth = |t: &str| format!("Bearer {t}");

    // Alice opens a DM with bob (participants = {alice, bob}).
    let dm: Value = ctx
        .client
        .post(format!("{base}/workspaces/{}/dm", ws.id.0))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"member_id": alice.id.0, "other_member_id": bob.id.0}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = dm["thread_id"].as_str().expect("dm thread_id");

    let get_msgs = |tok: &str| {
        ctx.client
            .get(format!("{base}/threads/{tid}/messages"))
            .header("Authorization", auth(tok))
            .send()
    };

    // Non-participant is denied via the generic thread route…
    assert_eq!(
        get_msgs(&mallory_tok).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    // …the generic thread GET too.
    assert_eq!(
        ctx.client
            .get(format!("{base}/threads/{tid}"))
            .header("Authorization", auth(&mallory_tok))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    // …participants (both members) still read it.
    assert_eq!(get_msgs(&alice_tok).await.unwrap().status(), StatusCode::OK);
    assert_eq!(get_msgs(&bob_tok).await.unwrap().status(), StatusCode::OK);

    ctx.server.abort();
}

/// Cluster 283: `ListTasks` lists the workspace's A2A tasks and drops those whose
/// context thread the caller cannot read (per-channel RBAC filter), and
/// `GetExtendedAgentCard` returns the card to an authenticated client.
#[tokio::test]
async fn a2a_list_tasks_filters_private_and_extended_card() {
    let ctx = spawn().await;
    let base = ctx.base();

    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "a2a-list".into(),
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
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
        }
    };
    let alice = mk("alice").await;
    let mallory = mk("mallory").await;
    let alice_tok = mint(ctx.store.as_ref(), ws.id, alice.id).await;
    let mallory_tok = mint(ctx.store.as_ref(), ws.id, mallory.id).await;
    let auth = |t: &str| format!("Bearer {t}");

    // A PUBLIC channel + thread, and a PRIVATE channel + thread (alice member of both).
    let mk_thread = |private: bool, name: &'static str, title: &'static str| {
        let client = ctx.client.clone();
        let base = base.clone();
        let tok = alice_tok.clone();
        let ws_id = ws.id.0;
        async move {
            let ch: Value = client
                .post(format!("{base}/workspaces/{ws_id}/channels"))
                .header("Authorization", auth(&tok))
                .json(&json!({"name": name, "private": private}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let cid = ch["id"].as_str().unwrap().to_string();
            let th: Value = client
                .post(format!("{base}/channels/{cid}/threads"))
                .header("Authorization", auth(&tok))
                .json(&json!({"title": title}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            th["id"].as_str().unwrap().to_string()
        }
    };
    let public_tid = mk_thread(false, "general", "pub-task").await;
    let private_tid = mk_thread(true, "secret", "priv-task").await;

    // Alice creates a task in each thread via A2A SendMessage.
    let send = |tid: &str| {
        let client = ctx.client.clone();
        let base = base.clone();
        let tok = alice_tok.clone();
        let member = alice.id.0;
        let tid = tid.to_string();
        async move {
            client
                .post(format!("{base}/a2a/v1/rpc"))
                .header("Authorization", auth(&tok))
                .json(&json!({
                    "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
                    "params": {
                        "message": {"role": "user", "parts": [{"type": "text", "text": "hi"}]},
                        "metadata": {"maidan": {"threadId": tid, "authorId": member}}
                    }
                }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    assert!(send(&public_tid).await["result"].is_object());
    assert!(send(&private_tid).await["result"].is_object());

    let list = |tok: String| {
        let client = ctx.client.clone();
        let base = base.clone();
        async move {
            client
                .post(format!("{base}/a2a/v1/rpc"))
                .header("Authorization", auth(&tok))
                .json(&json!({"jsonrpc": "2.0", "id": 2, "method": "ListTasks", "params": {}}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    // Alice (member of both channels) sees both tasks.
    let alice_list = list(alice_tok.clone()).await;
    assert_eq!(
        alice_list["result"]["tasks"].as_array().unwrap().len(),
        2,
        "alice sees both tasks"
    );

    // Mallory (not a member of the private channel) sees only the public task.
    let mallory_list = list(mallory_tok.clone()).await;
    let mallory_ctxs: Vec<&str> = mallory_list["result"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["contextId"].as_str().unwrap())
        .collect();
    assert_eq!(mallory_ctxs.len(), 1, "mallory sees only the public task");
    assert_eq!(mallory_ctxs[0], public_tid);

    // GetExtendedAgentCard returns a spec-shaped Agent Card to an authed client.
    let card: Value = ctx
        .client
        .post(format!("{base}/a2a/v1/rpc"))
        .header("Authorization", auth(&alice_tok))
        .json(&json!({"jsonrpc": "2.0", "id": 3, "method": "GetExtendedAgentCard"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let result = &card["result"];
    assert_eq!(result["name"].as_str(), Some("maidan"));
    assert_eq!(
        result["supportedInterfaces"][0]["protocolBinding"].as_str(),
        Some("JSONRPC")
    );
    assert_eq!(
        result["supportedInterfaces"][0]["protocolVersion"].as_str(),
        Some("1.0")
    );
    assert_eq!(
        result["capabilities"]["extendedAgentCard"].as_bool(),
        Some(true)
    );
    assert!(result["skills"].as_array().is_some_and(|s| !s.is_empty()));

    ctx.server.abort();
}
