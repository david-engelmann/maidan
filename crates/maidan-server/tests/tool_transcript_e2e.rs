//! Cluster 197: thread tool-call transcript over REST + MCP.
//!
//! Auth ENABLED. Proves the transcript correlates `ToolUse`/`ToolResult` blocks
//! across a thread's messages on both surfaces, and that a non-member of a
//! private channel is denied.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ContentBlock, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread,
    NewWorkspace, ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

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

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
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
        _server: server,
        client: reqwest::Client::new(),
        store,
        _dir: dir,
    }
}

fn auth(t: &str) -> String {
    format!("Bearer {t}")
}

/// Seed a thread whose messages carry a `ToolUse` and a later `ToolResult`.
async fn seed(ctx: &Ctx, ws: WorkspaceId, member: MemberId, private: bool) -> ThreadId {
    let store = ctx.store.clone();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: if private {
                "secret".into()
            } else {
                "work".into()
            },
            topic: None,
            private,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .unwrap();
    let mk = |blocks: Vec<ContentBlock>| {
        let store = store.clone();
        let tid = thread.id;
        async move {
            store
                .post_message(NewMessage {
                    thread_id: tid,
                    author_id: member,
                    body: maidan_types::derive_body(&blocks),
                    metadata: json!({}),
                    content: Some(blocks),
                })
                .await
                .unwrap()
        }
    };
    mk(vec![ContentBlock::ToolUse {
        id: "call-1".into(),
        name: "search".into(),
        input: json!({"q": "widgets"}),
    }])
    .await;
    mk(vec![ContentBlock::ToolResult {
        tool_use_id: "call-1".into(),
        content: "3 results".into(),
        is_error: false,
    }])
    .await;
    thread.id
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

#[tokio::test]
async fn tool_transcript_correlates_over_rest_and_mcp() {
    let ctx = spawn().await;
    let base = ctx.base();
    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let alice = mk_member(ctx.store.as_ref(), ws.id, "alice").await;
    let tok = mint(
        ctx.store.as_ref(),
        ws.id,
        alice,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
    )
    .await;
    let tid = seed(&ctx, ws.id, alice, false).await;

    // REST: the transcript correlates the ToolUse with its ToolResult.
    let rest: Value = ctx
        .client
        .get(format!("{base}/threads/{}/tool-transcript", tid.0))
        .header("Authorization", auth(&tok))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rest["thread_id"], json!(tid.0));
    let entries = rest["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "one tool call");
    assert_eq!(entries[0]["tool_use_id"], json!("call-1"));
    assert_eq!(entries[0]["name"], json!("search"));
    assert_eq!(entries[0]["result"]["content"], json!("3 results"));
    assert_eq!(entries[0]["result"]["is_error"], json!(false));

    // MCP: same transcript over tools/call.
    let mcp: Value = ctx
        .client
        .post(format!("{base}/mcp"))
        .header("Authorization", auth(&tok))
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "get_tool_transcript", "arguments": { "thread_id": tid.0 } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = mcp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["entries"][0]["tool_use_id"], json!("call-1"));
    assert_eq!(
        parsed["entries"][0]["result"]["content"],
        json!("3 results")
    );
}

#[tokio::test]
async fn non_member_is_denied_the_transcript_of_a_private_thread() {
    let ctx = spawn().await;
    let base = ctx.base();
    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let alice = mk_member(ctx.store.as_ref(), ws.id, "alice").await;
    let mallory = mk_member(ctx.store.as_ref(), ws.id, "mallory").await;
    // Alice owns the private thread (author). Mallory is not a channel member.
    let tid = seed(&ctx, ws.id, alice, true).await;
    let mallory_tok = mint(
        ctx.store.as_ref(),
        ws.id,
        mallory,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;

    let resp = ctx
        .client
        .get(format!("{base}/threads/{}/tool-transcript", tid.0))
        .header("Authorization", auth(&mallory_tok))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
