//! The A2A v1.0 agent end to end over HTTP: the JSON-RPC and HTTP+JSON
//! bindings against an auth-enabled server. The official TCK runs in
//! `scripts/a2a-tck.sh`; these tests pin Maidan's own semantics (contexts are
//! threads, the author is the caller, tasks hold no words) and the fixes the
//! TCK drove.

// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::collections::HashSet;
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use maidan_a2a::{A2aClient, Message, Part, Role, SendMessageRequest, SendMessageResponse};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime, WebhookRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations, A2aTaskWrite};
use maidan_types::{
    ApprovalGateId, ApprovalGateState, ChannelId, MemberId, MemberKind, MessageId, NewApiToken,
    NewApprovalGate, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const ALL_CAPS: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::MESSAGE_POST,
];

struct H {
    base: String,
    http: reqwest::Client,
    store: Arc<dyn Store>,
    ws: WorkspaceId,
    member: MemberId,
    token: String,
    _artifacts: tempfile::TempDir,
}

async fn spawn() -> H {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(256));
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth enabled
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.webhooks = WebhookRuntime::new(Some(Arc::new([7u8; 32])));
    state.a2a_card.public_origin = Some("https://maidan.example".into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let ws = store
        .create_workspace(NewWorkspace { name: "a2a".into() })
        .await
        .unwrap()
        .id;
    let member = member(store.as_ref(), ws, "caller").await;
    let token = mint(store.as_ref(), ws, member, ALL_CAPS).await;
    H {
        base: format!("http://{addr}"),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        ws,
        member,
        token,
        _artifacts: dir,
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
        })
        .await
        .unwrap()
        .id
}

fn message(text: &str) -> Value {
    json!({
        "messageId": uuid::Uuid::new_v4().to_string(),
        "role": "ROLE_USER",
        "parts": [{ "text": text }],
    })
}

impl H {
    async fn rpc_as(&self, token: &str, method: &str, params: Value) -> Value {
        self.http
            .post(format!("{}/a2a/v1/rpc", self.base))
            .bearer_auth(token)
            .header("A2A-Version", "1.0")
            .json(&json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn rpc(&self, method: &str, params: Value) -> Value {
        self.rpc_as(&self.token, method, params).await
    }

    /// Send `text` (into `context`, if given) and return the task.
    async fn send(&self, text: &str, context: Option<&str>) -> Value {
        let mut msg = message(text);
        if let Some(context) = context {
            msg["contextId"] = json!(context);
        }
        let resp = self.rpc("SendMessage", json!({ "message": msg })).await;
        assert!(resp.get("error").is_none(), "SendMessage failed: {resp}");
        resp["result"]["task"].clone()
    }

    async fn rest(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .http
            .request(method, format!("{}/a2a/v1{path}", self.base))
            .bearer_auth(&self.token)
            .header("A2A-Version", "1.0");
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.rest(reqwest::Method::GET, path, None).await
    }

    /// Open a pending approval gate in `thread_id`; its id is its task id.
    async fn gate(&self, thread_id: ThreadId) -> String {
        self.store
            .create_approval_gate(&NewApprovalGate {
                workspace_id: self.ws,
                thread_id: Some(thread_id),
                requested_by: self.member,
                prompt: "Deploy?".into(),
                schema: None,
            })
            .await
            .unwrap()
            .id
            .0
            .to_string()
    }

    /// Seed a task that has not finished, which SendMessage never produces.
    async fn working_task(&self, context: ThreadId) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        let at = chrono::Utc::now();
        self.store
            .upsert_a2a_task(A2aTaskWrite {
                workspace_id: self.ws,
                task_id: &id,
                context_id: Some(&context.0.to_string()),
                state: "TASK_STATE_WORKING",
                status_at: at,
                task_json: json!({
                    "id": id,
                    "contextId": context.0.to_string(),
                    "status": {
                        "state": "TASK_STATE_WORKING",
                        "timestamp": at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    },
                    "metadata": { "maidan": { "threadId": context.0 } },
                }),
            })
            .await
            .unwrap();
        id
    }
}

fn error_code(resp: &Value) -> i64 {
    resp["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("expected an error: {resp}"))
}

fn reason(resp: &Value) -> &str {
    resp["error"]["data"][0]["reason"]
        .as_str()
        .unwrap_or_default()
}

/// Every SSE `data:` payload of a response.
async fn sse(resp: reqwest::Response) -> Vec<Value> {
    assert!(resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream")));
    let mut body = String::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
    }
    body.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|d| serde_json::from_str(d.trim()).unwrap())
        .collect()
}

#[tokio::test]
async fn a_conversation_is_a_thread_the_caller_authors() {
    let h = spawn().await;
    let a2a = A2aClient::new(&h.base).unwrap().with_bearer(&h.token);

    // No contextId: a new thread in the workspace's `a2a` channel.
    let sent = a2a
        .send_message(SendMessageRequest {
            message: Message {
                message_id: "m-1".into(),
                context_id: None,
                task_id: None,
                role: Role::User,
                parts: vec![Part::text("hello from a2a")],
                metadata: Some(json!({ "trace": "abc" })),
                extensions: vec![],
                reference_task_ids: vec![],
            },
            configuration: None,
            metadata: None,
        })
        .await
        .unwrap();
    let SendMessageResponse::Task(task) = sent else {
        panic!("expected a task");
    };
    assert_eq!(task.status.state, "TASK_STATE_COMPLETED");
    assert!(task.status.timestamp.as_deref().unwrap().ends_with('Z'));
    let context = task.context_id.clone().unwrap();
    let history = task.history.clone().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].message_id, "m-1",
        "the client's messageId round-trips"
    );
    assert_eq!(history[0].metadata, Some(json!({ "trace": "abc" })));
    assert_eq!(history[0].task_id.as_deref(), Some(task.id.as_str()));

    let thread_id = ThreadId(uuid::Uuid::parse_str(&context).unwrap());
    let thread = h.store.get_thread(thread_id).await.unwrap();
    let channel = h.store.get_channel(thread.channel_id).await.unwrap();
    assert_eq!(channel.name, "a2a");
    assert!(!channel.private);
    let posted = h.store.list_messages(thread_id, 10).await.unwrap();
    assert_eq!(posted.len(), 1);
    assert_eq!(
        posted[0].author_id, h.member,
        "the token's member authors it"
    );
    assert_eq!(posted[0].body, "hello from a2a");

    // The context carries follow-ups into the same thread.
    let follow = h.send("a follow-up", Some(&context)).await;
    assert_eq!(follow["contextId"], json!(context));
    assert_eq!(h.store.list_messages(thread_id, 10).await.unwrap().len(), 2);

    // A client-chosen context binds to one new thread and keeps it.
    let first = h.send("client context", Some("conv-42")).await;
    let second = h.send("again", Some("conv-42")).await;
    assert_eq!(first["contextId"], json!("conv-42"));
    let thread_of = |t: &Value| t["metadata"]["maidan"]["threadId"].clone();
    assert_eq!(thread_of(&first), thread_of(&second));
    assert_ne!(thread_of(&first), json!(context));
    // Both land in the one `a2a` channel.
    let channels = h.store.list_channels(h.ws).await.unwrap();
    assert_eq!(channels.iter().filter(|c| c.name == "a2a").count(), 1);

    // GetTask renders history from the stored message; historyLength=0 omits it.
    let got = a2a.get_task(&task.id).await.unwrap();
    assert_eq!(
        got.history.unwrap()[0].parts,
        vec![Part::text("hello from a2a")]
    );
    let bare = h
        .rpc("GetTask", json!({ "id": task.id, "historyLength": 0 }))
        .await;
    assert!(bare["result"].get("history").is_none());
    let bare = h
        .rpc(
            "SendMessage",
            json!({ "message": message("quiet"), "configuration": { "historyLength": 0 } }),
        )
        .await;
    assert!(bare["result"]["task"].get("history").is_none());
}

#[tokio::test]
async fn a_context_may_name_a_thread_the_caller_can_read() {
    let h = spawn().await;
    let general = channel(h.store.as_ref(), h.ws, "general", false).await;
    let existing = thread(h.store.as_ref(), general).await;
    let task = h.send("into general", Some(&existing.0.to_string())).await;
    assert_eq!(task["metadata"]["maidan"]["threadId"], json!(existing.0));
    assert_eq!(h.store.list_messages(existing, 10).await.unwrap().len(), 1);

    // A private channel the caller is not in: refused, not silently re-routed.
    let secret = channel(h.store.as_ref(), h.ws, "secret", true).await;
    let hidden = thread(h.store.as_ref(), secret).await;
    let mut msg = message("sneak");
    msg["contextId"] = json!(hidden.0.to_string());
    let refused = h.rpc("SendMessage", json!({ "message": msg })).await;
    assert_eq!(error_code(&refused), -32000);
    assert!(h.store.list_messages(hidden, 10).await.unwrap().is_empty());

    // Another workspace's thread id is only an unknown context here.
    let other_ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .unwrap()
        .id;
    let foreign = thread(
        h.store.as_ref(),
        channel(h.store.as_ref(), other_ws, "x", false).await,
    )
    .await;
    let task = h.send("elsewhere", Some(&foreign.0.to_string())).await;
    assert_ne!(task["metadata"]["maidan"]["threadId"], json!(foreign.0));
    assert!(h.store.list_messages(foreign, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn refusals_carry_the_spec_error_codes() {
    let h = spawn().await;
    let done = h.send("done", None).await;
    let task_id = done["id"].as_str().unwrap();

    // Continuing a completed task: UnsupportedOperation, with ErrorInfo.
    let mut msg = message("more");
    msg["taskId"] = json!(task_id);
    let resp = h
        .rpc("SendMessage", json!({ "message": msg.clone() }))
        .await;
    assert_eq!(error_code(&resp), -32004);
    assert_eq!(reason(&resp), "UNSUPPORTED_OPERATION");
    assert_eq!(resp["error"]["data"][0]["domain"], "a2a-protocol.org");
    assert_eq!(
        resp["error"]["data"][0]["metadata"]["taskId"],
        json!(task_id)
    );
    // ... into another context: InvalidParams.
    msg["contextId"] = json!("somewhere-else");
    assert_eq!(
        error_code(&h.rpc("SendMessage", json!({ "message": msg })).await),
        -32602
    );
    // ... an unknown task: TaskNotFound.
    let mut msg = message("more");
    msg["taskId"] = json!("no-such-task");
    assert_eq!(
        error_code(&h.rpc("SendMessage", json!({ "message": msg })).await),
        -32001
    );

    // Unsupported part media: ContentTypeNotSupported (JSON-RPC and REST 415).
    let raw = json!({ "message": {
        "messageId": "m", "role": "ROLE_USER",
        "parts": [{ "raw": "dGNr", "mediaType": "application/x-unknown" }],
    } });
    let resp = h.rpc("SendMessage", raw.clone()).await;
    assert_eq!(error_code(&resp), -32005);
    let (status, body) = h
        .rest(reqwest::Method::POST, "/message:send", Some(raw))
        .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(body["error"]["status"], "INVALID_ARGUMENT");
    assert_eq!(
        body["error"]["details"][0]["reason"],
        "CONTENT_TYPE_NOT_SUPPORTED"
    );

    // A message without an id, or with no parts: InvalidParams.
    let resp = h
        .rpc("SendMessage", json!({ "message": { "role": "ROLE_USER", "parts": [{ "text": "x" }], "messageId": "" } }))
        .await;
    assert_eq!(error_code(&resp), -32602);

    // A token without message:post: PermissionDenied.
    let reader = mint(
        h.store.as_ref(),
        h.ws,
        h.member,
        &[capability::WORKSPACE_READ],
    )
    .await;
    let resp = h
        .rpc_as(&reader, "SendMessage", json!({ "message": message("x") }))
        .await;
    assert_eq!(error_code(&resp), -32000);
    // A token that may post but not create threads cannot open a context.
    let poster = mint(
        h.store.as_ref(),
        h.ws,
        h.member,
        &[capability::MESSAGE_POST],
    )
    .await;
    let resp = h
        .rpc_as(&poster, "SendMessage", json!({ "message": message("x") }))
        .await;
    assert_eq!(error_code(&resp), -32000);

    // Unknown tasks read as TaskNotFound on every binding.
    assert_eq!(
        error_code(&h.rpc("GetTask", json!({ "id": "nope" })).await),
        -32001
    );
    let (status, body) = h.get("/tasks/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["status"], "NOT_FOUND");
    assert_eq!(body["error"]["details"][0]["reason"], "TASK_NOT_FOUND");
    let resp = h
        .rpc("GetTask", json!({ "id": task_id, "historyLength": -1 }))
        .await;
    assert_eq!(error_code(&resp), -32602);
}

#[tokio::test]
async fn the_protocol_version_is_negotiated() {
    let h = spawn().await;
    let call = |version: Option<&str>, query: &str| {
        let mut req = h
            .http
            .post(format!("{}/a2a/v1/rpc{query}", h.base))
            .bearer_auth(&h.token)
            .json(&json!({ "jsonrpc": "2.0", "id": "v", "method": "ListTasks" }));
        if let Some(version) = version {
            req = req.header("A2A-Version", version);
        }
        async move { req.send().await.unwrap().json::<Value>().await.unwrap() }
    };
    for refused in [
        call(None, "").await,
        call(Some("0.3"), "").await,
        call(Some("2.0"), "").await,
    ] {
        assert_eq!(error_code(&refused), -32009, "{refused}");
        assert_eq!(refused["id"], "v");
    }
    assert!(call(Some("1.0.4"), "").await.get("result").is_some());
    assert!(call(None, "?A2A-Version=1.0").await.get("result").is_some());

    let resp = h
        .http
        .get(format!("{}/a2a/v1/tasks", h.base))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["error"]["details"][0]["reason"],
        "VERSION_NOT_SUPPORTED"
    );
    assert_eq!(body["error"]["status"], "UNIMPLEMENTED");
}

#[tokio::test]
async fn the_json_rpc_envelope_is_checked() {
    let h = spawn().await;
    let post = |path: &str, content_type: &str, body: &str| {
        h.http
            .post(format!("{}{path}", h.base))
            .bearer_auth(&h.token)
            .header("A2A-Version", "1.0")
            .header("content-type", content_type)
            .body(body.to_string())
            .send()
    };
    // The TCK posts to the advertised URL plus a trailing slash.
    let listed: Value = post(
        "/a2a/v1/rpc/",
        "application/json",
        r#"{"jsonrpc":"2.0","id":1,"method":"ListTasks"}"#,
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(listed["result"]["tasks"].is_array(), "{listed}");

    let parse: Value = post("/a2a/v1/rpc", "application/json", "{not json")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(error_code(&parse), -32700);
    assert_eq!(parse["id"], Value::Null);

    let resp = post(
        "/a2a/v1/rpc",
        "text/plain",
        r#"{"jsonrpc":"2.0","id":1,"method":"ListTasks"}"#,
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(error_code(&resp.json::<Value>().await.unwrap()), -32005);

    for (body, code) in [
        (r#"{"jsonrpc":"1.0","id":3,"method":"ListTasks"}"#, -32600),
        (r#"{"jsonrpc":"2.0","id":3,"method":"Nope"}"#, -32601),
        (
            r#"{"jsonrpc":"2.0","id":3,"method":"GetTask","params":{}}"#,
            -32602,
        ),
    ] {
        let resp: Value = post("/a2a/v1/rpc", "application/json", body)
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(error_code(&resp), code, "{body}");
        assert_eq!(resp["id"], 3);
    }
}

#[tokio::test]
async fn tasks_hold_no_words_and_history_follows_shredding() {
    let h = spawn().await;
    let task = h.send("a secret plan", None).await;
    let task_id = task["id"].as_str().unwrap();
    let row = h.store.get_a2a_task(task_id).await.unwrap().unwrap();
    let stored = row.task_json.to_string();
    assert!(
        !stored.contains("secret plan"),
        "task row holds words: {stored}"
    );
    assert!(row.task_json.get("history").is_none());

    let message_id: uuid::Uuid =
        serde_json::from_value(task["metadata"]["maidan"]["messageId"].clone()).unwrap();
    h.store
        .tombstone_message(MessageId(message_id))
        .await
        .unwrap();
    let got = h.rpc("GetTask", json!({ "id": task_id })).await;
    assert_eq!(got["result"]["status"]["state"], "TASK_STATE_COMPLETED");
    assert!(
        got["result"].get("history").is_none(),
        "a tombstoned message leaves no history: {got}"
    );
}

#[tokio::test]
async fn list_tasks_pages_filters_and_scopes() {
    let h = spawn().await;
    let a = h.send("a", Some("ctx-a")).await;
    let mut ids = vec![a["id"].as_str().unwrap().to_string()];
    for text in ["b", "c", "d"] {
        tokio::time::sleep(Duration::from_millis(3)).await;
        ids.push(
            h.send(text, Some("ctx-b")).await["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    ids.reverse(); // newest first

    // Keyset pages of two, then an empty token on the last page.
    let mut seen = Vec::new();
    let mut token = String::new();
    loop {
        let (status, page) = h.get(&format!("/tasks?pageSize=2&pageToken={token}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["pageSize"], 2);
        assert_eq!(page["totalSize"], 4);
        for t in page["tasks"].as_array().unwrap() {
            assert!(t.get("artifacts").is_none(), "artifacts omitted by default");
            seen.push(t["id"].as_str().unwrap().to_string());
        }
        token = page["nextPageToken"].as_str().unwrap().to_string();
        if token.is_empty() {
            break;
        }
    }
    assert_eq!(seen, ids, "every task once, newest status first");

    let by_context = h
        .rpc(
            "ListTasks",
            json!({ "contextId": "ctx-a", "includeArtifacts": true }),
        )
        .await;
    let tasks = by_context["result"]["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["artifacts"], json!([]));
    assert_eq!(by_context["result"]["totalSize"], 1);

    let since = h
        .rpc(
            "ListTasks",
            json!({ "statusTimestampAfter": a["status"]["timestamp"] }),
        )
        .await;
    assert_eq!(
        since["result"]["tasks"].as_array().unwrap().len(),
        4,
        "at or after"
    );
    let later = h
        .rpc(
            "ListTasks",
            json!({ "statusTimestampAfter": "2099-01-01T00:00:00Z", "historyLength": 0 }),
        )
        .await;
    assert!(later["result"]["tasks"].as_array().unwrap().is_empty());
    assert_eq!(later["result"]["nextPageToken"], "");

    for bad in [
        json!({ "pageSize": 0 }),
        json!({ "pageSize": 101 }),
        json!({ "status": "TASK_STATE_RUNNING" }),
        json!({ "statusTimestampAfter": "yesterday" }),
        json!({ "pageToken": "garbage" }),
        json!({ "historyLength": -5 }),
    ] {
        assert_eq!(
            error_code(&h.rpc("ListTasks", bad.clone()).await),
            -32602,
            "{bad}"
        );
    }
    let (status, _) = h.get("/tasks?pageSize=many").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Tasks in a private channel are invisible to, and uncounted for, others.
    let secret = channel(h.store.as_ref(), h.ws, "secret", true).await;
    let hidden = thread(h.store.as_ref(), secret).await;
    h.store
        .add_channel_member(secret, h.member, maidan_types::ChannelMemberRole::Member)
        .await
        .unwrap();
    h.send("private", Some(&hidden.0.to_string())).await;
    let outsider = member(h.store.as_ref(), h.ws, "outsider").await;
    let outsider_token = mint(h.store.as_ref(), h.ws, outsider, ALL_CAPS).await;
    let theirs = h.rpc_as(&outsider_token, "ListTasks", json!({})).await;
    assert_eq!(theirs["result"]["tasks"].as_array().unwrap().len(), 4);
    assert_eq!(theirs["result"]["totalSize"], 4);
    let mine = h.rpc("ListTasks", json!({})).await;
    assert_eq!(mine["result"]["totalSize"], 5);
}

#[tokio::test]
async fn cancel_and_subscribe_follow_task_state() {
    let h = spawn().await;
    let done = h.send("done", None).await;
    let done_id = done["id"].as_str().unwrap();
    let context = ThreadId(uuid::Uuid::parse_str(done["contextId"].as_str().unwrap()).unwrap());

    // A finished task is not cancelable, and there is nothing to subscribe to.
    let resp = h.rpc("CancelTask", json!({ "id": done_id })).await;
    assert_eq!(error_code(&resp), -32002);
    let (status, body) = h
        .rest(
            reqwest::Method::POST,
            &format!("/tasks/{done_id}:cancel"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["status"], "FAILED_PRECONDITION");
    assert_eq!(
        error_code(&h.rpc("SubscribeToTask", json!({ "id": done_id })).await),
        -32004
    );
    assert_eq!(
        error_code(&h.rpc("CancelTask", json!({ "id": "nope" })).await),
        -32001
    );
    let (status, _) = h
        .rest(reqwest::Method::POST, "/tasks/nope:subscribe", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A working task streams its state, then its end.
    let working = h.working_task(context).await;
    let stream = h
        .http
        .get(format!("{}/a2a/v1/tasks/{working}:subscribe", h.base))
        .bearer_auth(&h.token)
        .header("A2A-Version", "1.0")
        .send();
    let rpc_stream = h
        .http
        .post(format!("{}/a2a/v1/rpc", h.base))
        .bearer_auth(&h.token)
        .header("A2A-Version", "1.0")
        .json(&json!({ "jsonrpc": "2.0", "id": 9, "method": "SubscribeToTask", "params": { "id": working } }))
        .send();
    let (stream, rpc_stream) = tokio::join!(stream, rpc_stream);
    let (stream, rpc_stream) = (stream.unwrap(), rpc_stream.unwrap());
    tokio::time::sleep(Duration::from_millis(150)).await;
    let canceled = h.rpc("CancelTask", json!({ "id": working })).await;
    assert_eq!(canceled["result"]["status"]["state"], "TASK_STATE_CANCELED");

    let events = sse(stream).await;
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["task"]["status"]["state"], "TASK_STATE_WORKING");
    assert_eq!(
        events[1]["statusUpdate"]["status"]["state"],
        "TASK_STATE_CANCELED"
    );
    assert_eq!(events[1]["statusUpdate"]["taskId"], json!(working));
    let frames = sse(rpc_stream).await;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["id"], 9);
    assert_eq!(
        frames[1]["result"]["statusUpdate"]["status"]["state"],
        "TASK_STATE_CANCELED"
    );
}

#[tokio::test]
async fn streaming_send_works_over_both_bindings() {
    let h = spawn().await;
    let rpc = h
        .http
        .post(format!("{}/a2a/v1/rpc", h.base))
        .bearer_auth(&h.token)
        .header("A2A-Version", "1.0")
        .json(
            &json!({ "jsonrpc": "2.0", "id": "s", "method": "SendStreamingMessage",
                        "params": { "message": message("streamed") } }),
        )
        .send()
        .await
        .unwrap();
    let frames = sse(rpc).await;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["id"], "s");
    assert_eq!(
        frames[0]["result"]["task"]["status"]["state"],
        "TASK_STATE_WORKING"
    );
    let update = &frames[1]["result"]["statusUpdate"];
    assert_eq!(update["status"]["state"], "TASK_STATE_COMPLETED");
    assert_eq!(update["taskId"], frames[0]["result"]["task"]["id"]);
    assert!(update.get("final").is_none(), "v1.0 has no `final` flag");

    let rest = h
        .http
        .post(format!("{}/a2a/v1/message:stream", h.base))
        .bearer_auth(&h.token)
        .header("A2A-Version", "1.0")
        .json(&json!({ "message": message("streamed over rest") }))
        .send()
        .await
        .unwrap();
    let events = sse(rest).await;
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| e.as_object().unwrap().keys().next().unwrap().as_str())
        .collect();
    assert_eq!(kinds, ["task", "statusUpdate"]);

    let (status, _) = h
        .rest(
            reqwest::Method::POST,
            "/message:shout",
            Some(json!({ "message": message("x") })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

type Deliveries = Arc<Mutex<Vec<(axum::http::HeaderMap, Value)>>>;

async fn receiver() -> (String, Deliveries) {
    let seen: Deliveries = Arc::default();
    let sink = seen.clone();
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                let sink = sink.clone();
                async move {
                    sink.lock().unwrap().push((headers, body));
                    StatusCode::OK
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/hook"), seen)
}

#[tokio::test]
async fn push_configs_seal_secrets_and_deliver_the_task() {
    std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1");
    let h = spawn().await;
    let (hook, seen) = receiver().await;

    // Inline on SendMessage: registered before the task completes, so the
    // completion is delivered with the credentials.
    let sent = h
        .rpc(
            "SendMessage",
            json!({
                "message": message("notify me"),
                "configuration": { "taskPushNotificationConfig": {
                    "id": "inline", "url": hook, "token": "tok-1",
                    "authentication": { "scheme": "Bearer", "credentials": "cred-1" },
                } },
            }),
        )
        .await;
    let task_id = sent["result"]["task"]["id"].as_str().unwrap().to_string();
    for _ in 0..50 {
        if !seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    {
        let seen = seen.lock().unwrap();
        let (headers, body) = seen.first().expect("a delivery");
        assert_eq!(headers["authorization"], "Bearer cred-1");
        assert_eq!(headers["x-a2a-notification-token"], "tok-1");
        assert_eq!(body["task"]["id"], json!(task_id));
        assert_eq!(body["task"]["status"]["state"], "TASK_STATE_COMPLETED");
        assert!(body["task"].get("history").is_none());
    }

    // The stored row holds ciphertext only; responses never echo secrets.
    let row = h
        .store
        .get_a2a_task_push_config(&task_id, "inline")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.auth_scheme.as_deref(), Some("Bearer"));
    for sealed in [&row.token_ciphertext, &row.auth_credentials_ciphertext] {
        let sealed = sealed.as_deref().unwrap();
        assert!(!sealed.contains("tok-1") && !sealed.contains("cred-1"));
    }
    let got = h
        .rpc(
            "GetTaskPushNotificationConfig",
            json!({ "taskId": task_id, "id": "inline" }),
        )
        .await;
    assert_eq!(got["result"]["url"], json!(hook));
    assert_eq!(
        got["result"]["authentication"],
        json!({ "scheme": "Bearer" })
    );
    assert!(got["result"].get("token").is_none());

    // REST create/list/get/delete; delete is idempotent.
    let (status, created) = h
        .rest(
            reqwest::Method::POST,
            &format!("/tasks/{task_id}/pushNotificationConfigs"),
            Some(json!({ "url": "https://hooks.example/a2a", "token": "t2" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let config_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["taskId"], json!(task_id));
    assert!(created.get("token").is_none());
    let (_, listed) = h
        .get(&format!("/tasks/{task_id}/pushNotificationConfigs"))
        .await;
    assert_eq!(listed["configs"].as_array().unwrap().len(), 2);
    assert_eq!(listed["nextPageToken"], "");

    // Pages of one walk both configs in id order over each binding.
    let mut by_rest = Vec::new();
    let mut token = String::new();
    loop {
        let (status, page) = h
            .get(&format!(
                "/tasks/{task_id}/pushNotificationConfigs?pageSize=1&pageToken={token}"
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let configs = page["configs"].as_array().unwrap();
        assert_eq!(configs.len(), 1);
        by_rest.push(configs[0]["id"].as_str().unwrap().to_string());
        token = page["nextPageToken"].as_str().unwrap().to_string();
        if token.is_empty() {
            break;
        }
    }
    let mut expected = vec![config_id.clone(), "inline".to_string()];
    expected.sort();
    assert_eq!(by_rest, expected);
    let first = h
        .rpc(
            "ListTaskPushNotificationConfigs",
            json!({ "taskId": task_id, "pageSize": 1 }),
        )
        .await;
    let rest = h
        .rpc(
            "ListTaskPushNotificationConfigs",
            json!({ "taskId": task_id, "pageToken": first["result"]["nextPageToken"] }),
        )
        .await;
    assert_eq!(rest["result"]["configs"][0]["id"], json!(expected[1]));
    assert_eq!(rest["result"]["nextPageToken"], "");
    for bad in [
        json!({ "taskId": task_id, "pageSize": 0 }),
        json!({ "taskId": task_id, "pageSize": 101 }),
        json!({ "taskId": task_id, "pageToken": "not base64!" }),
    ] {
        assert_eq!(
            error_code(&h.rpc("ListTaskPushNotificationConfigs", bad.clone()).await),
            -32602,
            "{bad}"
        );
    }
    let (status, _) = h
        .get(&format!(
            "/tasks/{task_id}/pushNotificationConfigs?pageSize=some"
        ))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let path = format!("/tasks/{task_id}/pushNotificationConfigs/{config_id}");
    for _ in 0..2 {
        let (status, _) = h.rest(reqwest::Method::DELETE, &path, None).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, body) = h.get(&path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["details"][0]["reason"], "TASK_NOT_FOUND");

    // A path/body mismatch, a private target, and an unknown task.
    let (status, _) = h
        .rest(
            reqwest::Method::POST,
            &format!("/tasks/{task_id}/pushNotificationConfigs"),
            Some(json!({ "taskId": "other", "url": "https://hooks.example" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let resp = h
        .rpc(
            "CreateTaskPushNotificationConfig",
            json!({ "task_id": "nope", "url": "https://hooks.example" }),
        )
        .await;
    assert_eq!(error_code(&resp), -32001, "proto field names parse too");
}

/// Every task update goes to each of a task's push configs, so the list is
/// capped: past ten, a new config is refused, while re-sending an existing id
/// still replaces it.
#[tokio::test]
async fn a_task_holds_at_most_ten_push_configs() {
    let h = spawn().await;
    let sent = h
        .rpc("SendMessage", json!({ "message": message("cap me") }))
        .await;
    let task_id = sent["result"]["task"]["id"].as_str().unwrap().to_string();
    let create =
        |id: String| json!({ "taskId": task_id, "id": id, "url": "https://hooks.example/a2a" });
    for n in 0..10 {
        let resp = h
            .rpc("CreateTaskPushNotificationConfig", create(format!("c{n}")))
            .await;
        assert!(resp.get("error").is_none(), "config {n}: {resp}");
    }
    let refused = h
        .rpc("CreateTaskPushNotificationConfig", create("c10".into()))
        .await;
    assert_eq!(error_code(&refused), -32602, "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("at most 10")),
        "{refused}"
    );
    let replaced = h
        .rpc("CreateTaskPushNotificationConfig", create("c3".into()))
        .await;
    assert!(
        replaced.get("error").is_none(),
        "an existing id replaces: {replaced}"
    );
    let (status, _) = h
        .rest(
            reqwest::Method::DELETE,
            &format!("/tasks/{task_id}/pushNotificationConfigs/c0"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let room = h
        .rpc("CreateTaskPushNotificationConfig", create("c10".into()))
        .await;
    assert!(
        room.get("error").is_none(),
        "deleting one makes room: {room}"
    );
}

#[tokio::test]
async fn a_pending_gate_is_an_input_required_task() {
    let h = spawn().await;
    let general = channel(h.store.as_ref(), h.ws, "general", false).await;
    let gate_thread = thread(h.store.as_ref(), general).await;
    let real = h.send("real", None).await;
    let gate = h
        .store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: h.ws,
            thread_id: Some(gate_thread),
            requested_by: h.member,
            prompt: "Deploy to prod?".into(),
            schema: None,
        })
        .await
        .unwrap();
    let gate_id = gate.id.0.to_string();

    let got = h.rpc("GetTask", json!({ "id": gate_id })).await;
    assert_eq!(
        got["result"]["status"]["state"],
        "TASK_STATE_INPUT_REQUIRED"
    );
    assert_eq!(got["result"]["status"]["message"]["role"], "ROLE_AGENT");
    assert_eq!(
        got["result"]["status"]["message"]["parts"][0]["text"],
        "Deploy to prod?"
    );
    assert_eq!(got["result"]["contextId"], json!(gate_thread.0.to_string()));

    let only = h
        .rpc("ListTasks", json!({ "status": "input-required" }))
        .await;
    let tasks = only["result"]["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], json!(gate_id));
    let all = h.rpc("ListTasks", json!({})).await;
    assert_eq!(all["result"]["totalSize"], 2);
    let done = h
        .rpc("ListTasks", json!({ "status": "TASK_STATE_COMPLETED" }))
        .await;
    let done = done["result"]["tasks"].as_array().unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0]["id"], real["id"]);

    // A gate is answered through approvals, not A2A.
    assert_eq!(
        error_code(&h.rpc("CancelTask", json!({ "id": gate_id })).await),
        -32002
    );
    let mut msg = message("yes");
    msg["taskId"] = json!(gate_id);
    assert_eq!(
        error_code(&h.rpc("SendMessage", json!({ "message": msg })).await),
        -32004
    );
    let resp = h
        .rpc(
            "CreateTaskPushNotificationConfig",
            json!({ "taskId": gate_id, "url": "https://hooks.example" }),
        )
        .await;
    assert_eq!(error_code(&resp), -32003);

    h.store
        .resolve_approval_gate(gate.id, h.member, ApprovalGateState::Accepted, None)
        .await
        .unwrap();
    assert_eq!(
        error_code(&h.rpc("GetTask", json!({ "id": gate_id })).await),
        -32001
    );
}

/// Every pending gate is listed and counted, however many there are: they
/// page by the same keyset as stored tasks, merged into one order.
#[tokio::test]
async fn list_tasks_pages_through_every_pending_gate() {
    let h = spawn().await;
    let general = channel(h.store.as_ref(), h.ws, "general", false).await;
    let open = thread(h.store.as_ref(), general).await;
    let secret = channel(h.store.as_ref(), h.ws, "secret", true).await;
    let hidden = thread(h.store.as_ref(), secret).await;
    h.store
        .add_channel_member(secret, h.member, maidan_types::ChannelMemberRole::Member)
        .await
        .unwrap();
    // More gates than any fixed scan would reach, with tasks interleaved and
    // three gates only the caller can read.
    let mut created = HashSet::new();
    for i in 0..520 {
        created.insert(h.gate(open).await);
        if i % 200 == 0 {
            created.insert(h.working_task(open).await);
        }
    }
    let mut private = HashSet::new();
    for _ in 0..3 {
        private.insert(h.gate(hidden).await);
    }
    let newest = h.gate(hidden).await;
    private.insert(newest.clone());
    created.extend(private.iter().cloned());

    let mut seen: Vec<(String, String)> = Vec::new();
    let mut token = String::new();
    loop {
        let page = h
            .rpc("ListTasks", json!({ "pageSize": 100, "pageToken": token }))
            .await;
        assert_eq!(page["result"]["totalSize"], 527, "{page}");
        for task in page["result"]["tasks"].as_array().unwrap() {
            seen.push((
                task["status"]["timestamp"].as_str().unwrap().to_string(),
                task["id"].as_str().unwrap().to_string(),
            ));
        }
        token = page["result"]["nextPageToken"]
            .as_str()
            .unwrap()
            .to_string();
        if token.is_empty() {
            break;
        }
    }
    assert!(
        seen.windows(2).all(|w| w[0] > w[1]),
        "newest status first, ties by id"
    );
    let ids: Vec<String> = seen.into_iter().map(|(_, id)| id).collect();
    assert_eq!(ids.len(), created.len(), "nothing listed twice");
    assert_eq!(ids.iter().cloned().collect::<HashSet<_>>(), created);

    let gates_only = h
        .rpc(
            "ListTasks",
            json!({ "status": "TASK_STATE_INPUT_REQUIRED", "pageSize": 1 }),
        )
        .await;
    assert_eq!(gates_only["result"]["totalSize"], 524);
    let in_hidden = h
        .rpc("ListTasks", json!({ "contextId": hidden.0.to_string() }))
        .await;
    assert_eq!(in_hidden["result"]["totalSize"], 4);
    assert_eq!(in_hidden["result"]["tasks"].as_array().unwrap().len(), 4);
    let not_a_thread = h
        .rpc(
            "ListTasks",
            json!({ "contextId": hidden.0.to_string().to_uppercase() }),
        )
        .await;
    assert_eq!(not_a_thread["result"]["totalSize"], 0);

    // "At or after" a sub-millisecond instant: the gate stamped in the
    // millisecond before it is excluded, the newer ones kept.
    let at = h
        .store
        .get_approval_gate(ApprovalGateId(uuid::Uuid::parse_str(&newest).unwrap()))
        .await
        .unwrap()
        .unwrap()
        .created_at;
    let since = (at + chrono::Duration::microseconds(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    let after = h
        .rpc("ListTasks", json!({ "statusTimestampAfter": since }))
        .await;
    assert_eq!(after["result"]["totalSize"], 0, "{after}");
    let since = at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    let at_or_after = h
        .rpc("ListTasks", json!({ "statusTimestampAfter": since }))
        .await;
    assert!(at_or_after["result"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["id"] == json!(newest)));

    // Someone outside the private channel sees and counts only the rest.
    let outsider = member(h.store.as_ref(), h.ws, "outsider").await;
    let outsider_token = mint(h.store.as_ref(), h.ws, outsider, ALL_CAPS).await;
    let theirs = h
        .rpc_as(&outsider_token, "ListTasks", json!({ "pageSize": 5 }))
        .await;
    assert_eq!(theirs["result"]["totalSize"], 523);
    let first: Vec<_> = theirs["result"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    let visible: Vec<_> = ids.iter().filter(|id| !private.contains(*id)).collect();
    assert_eq!(first.iter().collect::<Vec<_>>(), visible[..5]);
}

#[tokio::test]
async fn the_agent_card_is_cacheable_and_declares_auth() {
    let h = spawn().await;
    let url = format!("{}/.well-known/agent-card.json", h.base);
    let resp = h.http.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let headers = resp.headers().clone();
    assert_eq!(headers["cache-control"], "public, max-age=300");
    assert!(headers.contains_key("last-modified"));
    assert_eq!(headers["content-type"], "application/json");
    let etag = headers["etag"].to_str().unwrap().to_string();
    let card: Value = resp.json().await.unwrap();
    assert_eq!(
        card["supportedInterfaces"],
        json!([
            { "url": "https://maidan.example/a2a/v1/rpc", "protocolBinding": "JSONRPC", "protocolVersion": "1.0" },
            { "url": "https://maidan.example/a2a/v1", "protocolBinding": "HTTP+JSON", "protocolVersion": "1.0" },
        ])
    );
    assert_eq!(
        card["securitySchemes"]["bearer"]["httpAuthSecurityScheme"]["scheme"],
        "Bearer"
    );
    assert_eq!(
        card["securityRequirements"],
        json!([{ "schemes": { "bearer": { "list": [] } } }])
    );
    assert_eq!(card["capabilities"]["extendedAgentCard"], true);

    let fresh = h
        .http
        .get(&url)
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(fresh.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(fresh.headers()["etag"], etag.as_str());
    let stale = h
        .http
        .get(&url)
        .header("if-none-match", "\"other\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::OK);

    // The extended card needs a credential; REST success is application/json.
    let (status, extended) = h.get("/extendedAgentCard").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(extended["name"], card["name"]);
    let anonymous = h
        .http
        .get(format!("{}/a2a/v1/extendedAgentCard", h.base))
        .header("A2A-Version", "1.0")
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}
