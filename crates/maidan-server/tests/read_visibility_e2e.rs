//! Every read the R09 audit named answers by one rule: a member of the same
//! workspace who is not in the private channel or the DM, and a member of
//! another workspace, each get nothing. REST and MCP both, with auth on.
//!
//! - `GET /dm/{id}/messages`, and every other `GET /dm/{id}/*` route;
//! - `GET /artifacts/{sha}` and `/meta`, MCP `get_artifact`,
//!   `get_artifact_metadata` and `resources/read` of `maidan://artifacts/{sha}`;
//! - `GET /workspaces/{wid}/approval-gates`, `POST /approval-gates/{id}/answer`,
//!   MCP `get_approval_gate` and `approval_decide`;
//! - `GET /members/{id}/inbox`, `/mentions`, `/notifications` and
//!   `/notifications/unread-count`.

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
    ApprovalRisk, ChannelId, ChannelMemberRole, EventKind, MemberId, MemberKind, NewApiToken,
    NewApprovalGate, NewChannel, NewMember, NewMessage, NewNotification, NewThread, NewWorkspace,
    ThreadId, WorkspaceId,
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
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn get(&self, token: &str, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }

    async fn mcp(&self, token: &str, method: &str, params: Value) -> Value {
        self.client
            .post(self.url("/mcp"))
            .bearer_auth(token)
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn tool(&self, token: &str, name: &str, arguments: Value) -> Value {
        self.mcp(
            token,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
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
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth enabled
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
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        _dir: dir,
    }
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

async fn token(store: &dyn Store, ws: WorkspaceId, member_id: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::ARTIFACT_UPLOAD.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn channel(store: &dyn Store, ws: WorkspaceId, name: &str, private: bool) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: name.into(),
            topic: None,
            private,
        })
        .await
        .unwrap()
        .id
}

async fn thread(store: &dyn Store, channel_id: ChannelId) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id
}

/// alice and bob share a DM and a private channel; carol is in the same
/// workspace and in neither; eve belongs to another workspace.
struct World {
    ctx: Ctx,
    ws: WorkspaceId,
    alice: MemberId,
    carol: MemberId,
    eve: MemberId,
    alice_tok: String,
    carol_tok: String,
    eve_tok: String,
    private_thread: ThreadId,
    open_thread: ThreadId,
    dm_id: uuid::Uuid,
    dm_thread: ThreadId,
}

async fn world() -> World {
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
            name: "other".into(),
        })
        .await
        .unwrap()
        .id;
    let alice = member(store, ws, "alice").await;
    let bob = member(store, ws, "bob").await;
    let carol = member(store, ws, "carol").await;
    let eve = member(store, other, "eve").await;
    let private = channel(store, ws, "board", true).await;
    for m in [alice, bob] {
        store
            .add_channel_member(private, m, ChannelMemberRole::Member)
            .await
            .unwrap();
    }
    let public = channel(store, ws, "general", false).await;
    let private_thread = thread(store, private).await;
    let open_thread = thread(store, public).await;
    let dm = store.open_dm_conversation(ws, alice, bob).await.unwrap();
    World {
        alice_tok: token(store, ws, alice).await,
        carol_tok: token(store, ws, carol).await,
        eve_tok: token(store, other, eve).await,
        ws,
        alice,
        carol,
        eve,
        private_thread,
        open_thread,
        dm_id: dm.id.0,
        dm_thread: dm.thread_id,
        ctx,
    }
}

/// The JSON a tool call returned as its text content, or the error object.
fn tool_result(response: &Value) -> Result<Value, Value> {
    if let Some(error) = response.get("error") {
        return Err(error.clone());
    }
    let result = &response["result"];
    if result["isError"] == json!(true) {
        return Err(result.clone());
    }
    let text = result["content"][0]["text"].as_str().unwrap_or("null");
    Ok(serde_json::from_str(text).unwrap_or(Value::Null))
}

#[tokio::test]
async fn only_a_dms_participants_read_its_messages() {
    let w = world().await;
    w.ctx
        .store
        .post_message(NewMessage {
            thread_id: w.dm_thread,
            author_id: w.alice,
            body: "the offer is 42".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();

    // Every GET under /dm/{id}, so a sibling added later is held to it too.
    for path in [
        format!("/dm/{}", w.dm_id),
        format!("/dm/{}/messages", w.dm_id),
    ] {
        let carol = w.ctx.get(&w.carol_tok, &path).await;
        assert_eq!(carol.status(), StatusCode::FORBIDDEN, "carol: {path}");
        let eve = w.ctx.get(&w.eve_tok, &path).await;
        assert!(
            matches!(eve.status(), StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "another workspace: {path} answered {}",
            eve.status()
        );
        let alice = w.ctx.get(&w.alice_tok, &path).await;
        assert_eq!(alice.status(), StatusCode::OK, "a participant: {path}");
    }
    let messages: Value = w
        .ctx
        .get(&w.alice_tok, &format!("/dm/{}/messages", w.dm_id))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(messages[0]["body"], "the offer is 42");
}

#[tokio::test]
async fn an_artifact_linked_in_a_private_channel_is_read_only_by_its_members() {
    let w = world().await;
    let upload = |body: &'static [u8]| {
        let req = w
            .ctx
            .client
            .post(w.ctx.url("/artifacts?kind=attachment&filename=board.txt"))
            .bearer_auth(&w.alice_tok)
            .body(body);
        async move {
            let resp = req.send().await.unwrap();
            assert_eq!(resp.status(), StatusCode::CREATED);
            resp.json::<Value>().await.unwrap()["sha256"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let private_sha = upload(b"the acquisition price is 42").await;
    let public_sha = upload(b"the lunch menu").await;
    for (tid, sha) in [
        (w.private_thread, &private_sha),
        (w.open_thread, &public_sha),
    ] {
        let linked = w
            .ctx
            .client
            .put(w.ctx.url(&format!("/threads/{}/artifacts/{sha}", tid.0)))
            .bearer_auth(&w.alice_tok)
            .send()
            .await
            .unwrap();
        assert!(linked.status().is_success(), "link: {}", linked.status());
    }
    // Linking it somewhere carol can read does not unlock it.
    let relinked = w
        .ctx
        .client
        .put(w.ctx.url(&format!(
            "/threads/{}/artifacts/{private_sha}",
            w.open_thread.0
        )))
        .bearer_auth(&w.carol_tok)
        .send()
        .await
        .unwrap();
    assert!(relinked.status().is_success(), "{}", relinked.status());

    for suffix in ["", "/meta"] {
        let path = format!("/artifacts/{private_sha}{suffix}");
        for (who, tok) in [("carol", &w.carol_tok), ("eve", &w.eve_tok)] {
            let resp = w.ctx.get(tok, &path).await;
            assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{who}: {path}");
        }
        let alice = w.ctx.get(&w.alice_tok, &path).await;
        assert_eq!(alice.status(), StatusCode::OK, "alice: {path}");
        let public = w
            .ctx
            .get(&w.carol_tok, &format!("/artifacts/{public_sha}{suffix}"))
            .await;
        assert_eq!(public.status(), StatusCode::OK, "a public channel's file");
    }

    for tool in ["get_artifact", "get_artifact_metadata"] {
        for (who, tok) in [("carol", &w.carol_tok), ("eve", &w.eve_tok)] {
            let response = w
                .ctx
                .tool(tok, tool, json!({ "sha256": private_sha }))
                .await;
            assert!(tool_result(&response).is_err(), "{who}: {tool}: {response}");
        }
        let response = w
            .ctx
            .tool(&w.alice_tok, tool, json!({ "sha256": private_sha }))
            .await;
        assert!(tool_result(&response).is_ok(), "alice: {tool}: {response}");
    }
    let uri = format!("maidan://artifacts/{private_sha}");
    for (who, tok) in [("carol", &w.carol_tok), ("eve", &w.eve_tok)] {
        let response = w
            .ctx
            .mcp(tok, "resources/read", json!({ "uri": uri }))
            .await;
        assert!(
            response.get("error").is_some(),
            "{who}: resources/read: {response}"
        );
    }
    let response = w
        .ctx
        .mcp(&w.alice_tok, "resources/read", json!({ "uri": uri }))
        .await;
    assert!(response.get("error").is_none(), "alice: {response}");
}

#[tokio::test]
async fn a_gate_on_a_thread_is_listed_and_answered_only_by_its_readers() {
    let w = world().await;
    let mut gates = Vec::new();
    for (thread_id, prompt) in [
        (Some(w.private_thread), "ship the board deck"),
        (Some(w.dm_thread), "send the offer"),
        (Some(w.open_thread), "merge the fix"),
    ] {
        gates.push(
            w.ctx
                .store
                .create_approval_gate(&NewApprovalGate {
                    workspace_id: w.ws,
                    thread_id,
                    requested_by: w.alice,
                    prompt: prompt.into(),
                    schema: None,
                    risk: ApprovalRisk::High,
                })
                .await
                .unwrap(),
        );
    }
    let listed = |tok: String| {
        let path = format!("/workspaces/{}/approval-gates", w.ws.0);
        let ctx = &w.ctx;
        async move {
            let resp = ctx.get(&tok, &path).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let views: Vec<Value> = resp.json().await.unwrap();
            views
                .into_iter()
                .map(|v| {
                    (
                        v["gate"]["prompt"].as_str().unwrap_or_default().to_string(),
                        v["request_state"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect::<Vec<_>>()
        }
    };
    let carol_sees: Vec<_> = listed(w.carol_tok.clone())
        .await
        .into_iter()
        .map(|(prompt, _)| prompt)
        .collect();
    assert_eq!(carol_sees, ["merge the fix"]);
    // alice reads all three, and her listing hands out each gate's
    // request_state: carol holding one (it leaked) must still get nowhere.
    let alice_sees = listed(w.alice_tok.clone()).await;
    assert_eq!(alice_sees.len(), 3);
    let eve = w
        .ctx
        .get(
            &w.eve_tok,
            &format!("/workspaces/{}/approval-gates", w.ws.0),
        )
        .await;
    assert!(
        matches!(eve.status(), StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "another workspace lists none: {}",
        eve.status()
    );

    // A gate carol cannot read is not found to her, whatever she answers.
    for gate in &gates[..2] {
        let state = alice_sees
            .iter()
            .find(|(prompt, _)| *prompt == gate.prompt)
            .map(|(_, state)| state.clone())
            .unwrap();
        for (who, tok) in [("carol", &w.carol_tok), ("eve", &w.eve_tok)] {
            let resp = w
                .ctx
                .client
                .post(w.ctx.url(&format!("/approval-gates/{}/answer", gate.id.0)))
                .bearer_auth(tok)
                .json(&json!({ "action": "decline", "request_state": state }))
                .send()
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::NOT_FOUND,
                "{who}: answer {}",
                gate.prompt
            );
            let got = w
                .ctx
                .tool(tok, "get_approval_gate", json!({ "gate_id": gate.id }))
                .await;
            assert_eq!(
                tool_result(&got),
                Ok(Value::Null),
                "{who}: get {}",
                gate.prompt
            );
            let decided = w
                .ctx
                .tool(
                    tok,
                    "approval_decide",
                    json!({ "gate_id": gate.id, "decision": "decline" }),
                )
                .await;
            assert!(
                tool_result(&decided).is_err(),
                "{who}: decide {}",
                gate.prompt
            );
        }
        let still = w
            .ctx
            .store
            .get_approval_gate(gate.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!still.state.is_resolved(), "nobody outside resolved it");
        let got = w
            .ctx
            .tool(
                &w.alice_tok,
                "get_approval_gate",
                json!({ "gate_id": gate.id }),
            )
            .await;
        assert_eq!(tool_result(&got).unwrap()["prompt"], gate.prompt.as_str());
    }
}

#[tokio::test]
async fn a_members_inbox_and_notifications_hold_only_what_it_can_read() {
    let w = world().await;
    let store = w.ctx.store.as_ref();
    for (source, (thread_id, body)) in (1..).zip([
        (w.private_thread, "@carol the price is 42"),
        (w.dm_thread, "@carol psst"),
        (w.open_thread, "@carol hello"),
    ]) {
        let message = store
            .post_message(NewMessage {
                thread_id,
                author_id: w.alice,
                body: body.into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
        store.record_mention(message.id, w.carol).await.unwrap();
        store
            .create_notification(NewNotification {
                workspace_id: w.ws,
                member_id: w.carol,
                kind: EventKind::MessagePosted,
                source_log_id: source,
                channel_id: Some(store.get_thread(thread_id).await.unwrap().channel_id),
                thread_id: Some(thread_id),
                message_id: Some(message.id),
                actor_id: Some(w.alice),
            })
            .await
            .unwrap();
    }
    // A notification written for eve about this workspace's thread.
    store
        .create_notification(NewNotification {
            workspace_id: w.ws,
            member_id: w.eve,
            kind: EventKind::MessagePosted,
            source_log_id: 99,
            channel_id: None,
            thread_id: Some(w.open_thread),
            message_id: None,
            actor_id: None,
        })
        .await
        .unwrap();

    let carol = w.carol.0;
    let inbox: Value = w
        .ctx
        .get(&w.carol_tok, &format!("/members/{carol}/inbox"))
        .await
        .json()
        .await
        .unwrap();
    let bodies: Vec<_> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["message_body"].as_str().unwrap())
        .collect();
    assert_eq!(bodies, ["@carol hello"]);
    assert_eq!(inbox["unread_count"], 1);
    let mentions: Vec<Value> = w
        .ctx
        .get(&w.carol_tok, &format!("/members/{carol}/mentions"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(mentions.len(), 1);
    let notes: Vec<Value> = w
        .ctx
        .get(&w.carol_tok, &format!("/members/{carol}/notifications"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["thread_id"], json!(w.open_thread));
    let unread: Value = w
        .ctx
        .get(
            &w.carol_tok,
            &format!("/members/{carol}/notifications/unread-count"),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(unread["count"], 1, "{unread}");
    let mcp_inbox = w
        .ctx
        .tool(&w.carol_tok, "get_inbox", json!({ "member_id": carol }))
        .await;
    assert_eq!(
        tool_result(&mcp_inbox).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "MCP reads the same inbox"
    );

    let eve = w.eve.0;
    let eve_notes: Vec<Value> = w
        .ctx
        .get(&w.eve_tok, &format!("/members/{eve}/notifications"))
        .await
        .json()
        .await
        .unwrap();
    assert!(
        eve_notes.is_empty(),
        "another workspace reads none of this one's"
    );
}
