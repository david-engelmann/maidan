//! A member deactivated through SCIM, or whose token is revoked, loses their
//! open server-sent event streams. This mirrors what #1350 does for
//! WebSockets. A stream is authorized once, when it opens, and makes no
//! further requests, so on its own it outlived the credential. Each stream ends
//! with one `stream_ended` event that gives the reason, then closes.
//!
//! Two tenants: deactivating Alice in workspace A ends Alice's streams and
//! nobody else's, in A or in B. The A2A `SubscribeToTask` streams are covered
//! on all three bindings: the JSON-RPC and REST ones end with `stream_ended`
//! like the rest, and the gRPC one ends with an `UNAUTHENTICATED` status,
//! which is that binding's terminal frame.

use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::{Stream, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::a2a_grpc::generated::{
    a2a_service_client::A2aServiceClient, stream_response, StreamResponse, SubscribeToTaskRequest,
    TaskState,
};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations, A2aTaskWrite};
use maidan_types::{
    AuditScope, MemberId, MemberKind, NewApiToken, NewAuditEvent, NewMember, NewWorkspace,
    WorkspaceId,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

/// Longer than the server's gap between two checks of a stream's credential.
const PAST_RECHECK_GAP: Duration = Duration::from_millis(1200);
/// Longer than the server's idle-stream check interval.
const PAST_RECHECK_TICK: Duration = Duration::from_secs(13);
const SESSION_REVISION: &str = "2024-11-05";
const STREAM_ENDED: &str = "stream_ended";

struct Env {
    base: String,
    grpc_base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Env {
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
    let grpc_search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::with_capacity(256));
    let mut state = AppState::new(
        store.clone(),
        artifacts.clone(),
        bus.clone(),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The A2A gRPC binding over the same store, so a deactivation lands on
    // its subscriptions too.
    let grpc_state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        grpc_search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    let svc = maidan_server::a2a_grpc::service(grpc_state);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .layer(maidan_server::trace_context::GrpcTraceLayer)
            .add_service(svc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                grpc_listener,
            ))
            .await
            .unwrap();
    });

    Env {
        base: format!("http://{addr}"),
        grpc_base: format!("http://{grpc_addr}"),
        // No client-level timeout: the SSE responses stay open.
        client: reqwest::Client::new(),
        store,
        _dir: dir,
    }
}

impl Env {
    async fn workspace(&self, name: &str) -> WorkspaceId {
        self.store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap()
            .id
    }

    /// A SCIM-provisioned member of `ws` and a token for them that can read,
    /// subscribe and create channels.
    async fn member(&self, ws: WorkspaceId, handle: &str) -> (MemberId, String) {
        let member = self
            .store
            .create_member(NewMember {
                workspace_id: ws,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap()
            .id;
        self.store
            .create_scim_user(member, ws, None, true)
            .await
            .unwrap();
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: ws,
                member_id: member,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: vec![
                    capability::WORKSPACE_READ.into(),
                    capability::WORKSPACE_WRITE.into(),
                    capability::EVENT_SUBSCRIBE.into(),
                    capability::CHANNEL_ADMIN.into(),
                    capability::MESSAGE_POST.into(),
                ],
                expires_at: None,
            })
            .await
            .unwrap();
        (member, secret.as_str().to_string())
    }

    async fn get_sse(&self, path: &str, token: &str) -> Sse {
        let resp = self
            .client
            .get(format!("{}{path}", self.base))
            .bearer_auth(token)
            .header("accept", "text/event-stream")
            .header("mcp-protocol-version", SESSION_REVISION)
            .send()
            .await
            .unwrap();
        Sse::open(resp)
    }

    /// `initialize` over `POST /mcp/streamable`: the response is the session's
    /// SSE stream.
    async fn streamable_session(&self, token: &str) -> Sse {
        let resp = self
            .client
            .post(format!("{}/mcp/streamable", self.base))
            .bearer_auth(token)
            .header("mcp-protocol-version", SESSION_REVISION)
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
            .send()
            .await
            .unwrap();
        Sse::open(resp)
    }

    /// Seed a task in `TASK_STATE_WORKING` in `ws`: the state a subscription
    /// streams until the task ends, so the stream stays open.
    async fn working_task(&self, ws: WorkspaceId, id: &str) {
        let at = chrono::Utc::now();
        let context = format!("ctx-{id}");
        self.store
            .upsert_a2a_task(A2aTaskWrite {
                workspace_id: ws,
                task_id: id,
                context_id: Some(&context),
                thread_id: None,
                state: "TASK_STATE_WORKING",
                status_at: at,
                task_json: json!({
                    "id": id,
                    "contextId": context,
                    "status": {
                        "state": "TASK_STATE_WORKING",
                        "timestamp": at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    },
                }),
            })
            .await
            .unwrap();
    }

    /// `SubscribeToTask` over the JSON-RPC binding: the response is the
    /// task's SSE stream.
    async fn a2a_rpc_subscribe(&self, task_id: &str, token: &str) -> Sse {
        let resp = self
            .client
            .post(format!("{}/a2a/v1/rpc", self.base))
            .bearer_auth(token)
            .header("A2A-Version", "1.0")
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "SubscribeToTask", "params": { "id": task_id }}))
            .send()
            .await
            .unwrap();
        Sse::open(resp)
    }

    /// `SubscribeToTask` over the REST binding: the response is the task's
    /// SSE stream.
    async fn a2a_rest_subscribe(&self, task_id: &str, token: &str) -> Sse {
        let resp = self
            .client
            .get(format!("{}/a2a/v1/tasks/{task_id}:subscribe", self.base))
            .bearer_auth(token)
            .header("A2A-Version", "1.0")
            .send()
            .await
            .unwrap();
        Sse::open(resp)
    }

    /// `SubscribeToTask` over the gRPC binding.
    async fn a2a_grpc_subscribe(
        &self,
        task_id: &str,
        token: &str,
    ) -> tonic::Streaming<StreamResponse> {
        let mut client = A2aServiceClient::connect(self.grpc_base.clone())
            .await
            .unwrap();
        let mut request = tonic::Request::new(SubscribeToTaskRequest {
            id: task_id.into(),
            ..Default::default()
        });
        request
            .metadata_mut()
            .insert("a2a-version", "1.0".parse().unwrap());
        request
            .metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        client
            .subscribe_to_task(request)
            .await
            .unwrap()
            .into_inner()
    }

    async fn create_channel(&self, ws: WorkspaceId, token: &str, name: &str) {
        let resp = self
            .client
            .post(format!("{}/workspaces/{}/channels", self.base, ws.0))
            .bearer_auth(token)
            .json(&json!({ "name": name }))
            .send()
            .await
            .unwrap();
        assert!(
            resp.status().is_success(),
            "create channel: {}",
            resp.status()
        );
    }

    /// Deactivate through the audited SCIM path, which also revokes the
    /// member's tokens.
    async fn deactivate(&self, ws: WorkspaceId, member: MemberId) {
        self.store
            .scim_update_user_audited(
                ws,
                member,
                None,
                None,
                false,
                NewAuditEvent {
                    scope: AuditScope::Workspace(ws),
                    actor_id: None,
                    action: "scim.user.update".into(),
                    target_kind: Some("member".into()),
                    target_id: Some(member.0),
                    metadata: json!({ "active": false }),
                },
            )
            .await
            .unwrap()
            .expect("SCIM user");
    }
}

#[derive(Debug)]
enum Frame {
    /// One SSE event: its `event:` name (if any) and its `data:`.
    Event(Option<String>, String),
    Closed,
    Silent,
}

struct Sse {
    body: Pin<Box<dyn Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    buf: String,
}

impl Sse {
    fn open(resp: reqwest::Response) -> Self {
        assert!(
            resp.status().is_success(),
            "stream refused: {}",
            resp.status()
        );
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            content_type.starts_with("text/event-stream"),
            "{content_type}"
        );
        Self {
            body: Box::pin(resp.bytes_stream()),
            buf: String::new(),
        }
    }

    /// The next event (keep-alive comments skipped), waiting at most `wait`.
    async fn next(&mut self, wait: Duration) -> Frame {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            while let Some(idx) = self.buf.find("\n\n") {
                let raw: String = self.buf.drain(..idx + 2).collect();
                let mut name = None;
                let mut data = Vec::new();
                for line in raw.lines() {
                    if let Some(v) = line.strip_prefix("event:") {
                        name = Some(v.trim().to_string());
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data.push(v.trim_start().to_string());
                    }
                }
                if name.is_some() || !data.is_empty() {
                    return Frame::Event(name, data.join("\n"));
                }
            }
            match tokio::time::timeout_at(deadline, self.body.next()).await {
                Err(_) => return Frame::Silent,
                Ok(None) | Ok(Some(Err(_))) => return Frame::Closed,
                Ok(Some(Ok(chunk))) => self.buf.push_str(&String::from_utf8_lossy(&chunk)),
            }
        }
    }

    /// Read until an event's data contains `needle`, the stream ends, or it
    /// goes quiet for `wait`.
    async fn until(&mut self, needle: &str, wait: Duration) -> Frame {
        loop {
            match self.next(wait).await {
                Frame::Event(name, data)
                    if data.contains(needle) || name.as_deref() == Some(STREAM_ENDED) =>
                {
                    return Frame::Event(name, data);
                }
                Frame::Event(..) => {}
                other => return other,
            }
        }
    }

    /// The stream must end with `stream_ended` naming `reason`, then close.
    async fn assert_ended(&mut self, wait: Duration, reason: &str) {
        match self.until(STREAM_ENDED, wait).await {
            Frame::Event(Some(name), data) if name == STREAM_ENDED => {
                let v: Value = serde_json::from_str(&data).unwrap();
                assert_eq!(v["reason"], reason, "{data}");
            }
            other => panic!("expected a stream_ended {reason:?} event, got {other:?}"),
        }
        match self.next(Duration::from_secs(3)).await {
            Frame::Closed => {}
            other => panic!("the stream should close after stream_ended, got {other:?}"),
        }
    }

    /// The stream must deliver an event containing `needle` and not end.
    async fn assert_carries(&mut self, needle: &str) {
        match self.until(needle, Duration::from_secs(5)).await {
            Frame::Event(name, data) if name.as_deref() != Some("stream_ended") => {
                assert!(data.contains(needle), "{data}");
            }
            other => panic!("expected an event with {needle:?}, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn deactivating_alice_in_a_ends_only_her_streams_at_their_next_frame() {
    let env = spawn().await;
    let a = env.workspace("Alpha").await;
    let b = env.workspace("Bravo").await;
    let (alice, alice_token) = env.member(a, "alice").await;
    let (_carol, carol_token) = env.member(a, "carol").await;
    let (_bob, bob_token) = env.member(b, "bob").await;

    let mut alice_stream = env
        .get_sse(&format!("/mcp/stream?workspace_id={}", a.0), &alice_token)
        .await;
    let mut carol_stream = env
        .get_sse(&format!("/mcp/stream?workspace_id={}", a.0), &carol_token)
        .await;
    let mut bob_stream = env
        .get_sse(&format!("/mcp/stream?workspace_id={}", b.0), &bob_token)
        .await;

    env.deactivate(a, alice).await;
    tokio::time::sleep(PAST_RECHECK_GAP).await;

    // Carol's new channel is the next frame bound for Alice's stream: it ends
    // instead of carrying it. Deactivation revoked Alice's token, which the
    // recheck sees first.
    env.create_channel(a, &carol_token, "after-alice").await;
    alice_stream
        .assert_ended(Duration::from_secs(5), "token no longer valid")
        .await;

    // Nobody else's stream ends: Carol in the same workspace, Bob in another.
    carol_stream.assert_carries("after-alice").await;
    env.create_channel(b, &bob_token, "bravo-news").await;
    bob_stream.assert_carries("bravo-news").await;
}

/// One read from a gRPC stream: an event, the status it failed with, its
/// end, or silence for the whole wait. The event is boxed — a
/// `StreamResponse` dwarfs the other variants.
enum GrpcRead {
    Event(Box<StreamResponse>),
    Failed(tonic::Status),
    Closed,
    Silent,
}

async fn grpc_read(stream: &mut tonic::Streaming<StreamResponse>, wait: Duration) -> GrpcRead {
    match tokio::time::timeout(wait, stream.message()).await {
        Err(_) => GrpcRead::Silent,
        Ok(Ok(Some(event))) => GrpcRead::Event(Box::new(event)),
        Ok(Ok(None)) => GrpcRead::Closed,
        Ok(Err(status)) => GrpcRead::Failed(status),
    }
}

#[tokio::test]
async fn deactivating_alice_ends_her_a2a_subscriptions_on_every_binding_and_nobody_elses() {
    let env = spawn().await;
    let a = env.workspace("Alpha").await;
    let b = env.workspace("Bravo").await;
    let (alice, alice_token) = env.member(a, "alice").await;
    let (_carol, carol_token) = env.member(a, "carol").await;
    let (_bob, bob_token) = env.member(b, "bob").await;
    env.working_task(a, "alice-task").await;
    env.working_task(a, "carol-task").await;
    env.working_task(b, "bob-task").await;

    let mut alice_rpc = env.a2a_rpc_subscribe("alice-task", &alice_token).await;
    let mut alice_rest = env.a2a_rest_subscribe("alice-task", &alice_token).await;
    let mut alice_grpc = env.a2a_grpc_subscribe("alice-task", &alice_token).await;
    let mut carol_rest = env.a2a_rest_subscribe("carol-task", &carol_token).await;
    let mut bob_rest = env.a2a_rest_subscribe("bob-task", &bob_token).await;

    // Every subscription opens with the working task.
    alice_rpc.assert_carries("TASK_STATE_WORKING").await;
    alice_rest.assert_carries("TASK_STATE_WORKING").await;
    match grpc_read(&mut alice_grpc, Duration::from_secs(5)).await {
        GrpcRead::Event(event) => match event.payload {
            Some(stream_response::Payload::Task(task)) => {
                assert_eq!(
                    task.status.expect("task status").state(),
                    TaskState::Working
                );
            }
            other => panic!("expected the working task, got {other:?}"),
        },
        GrpcRead::Failed(status) => panic!("the subscription failed at once: {status}"),
        GrpcRead::Closed => panic!("the subscription closed before its first event"),
        GrpcRead::Silent => panic!("the subscription sent no first event"),
    }
    carol_rest.assert_carries("TASK_STATE_WORKING").await;
    bob_rest.assert_carries("TASK_STATE_WORKING").await;

    env.deactivate(a, alice).await;

    // No frame is bound for Alice's subscriptions: each ends on the idle
    // re-check. The SSE bindings end with `stream_ended`; the gRPC one ends
    // with the `UNAUTHENTICATED` status that is its terminal frame.
    alice_rpc
        .assert_ended(PAST_RECHECK_TICK, "token no longer valid")
        .await;
    alice_rest
        .assert_ended(PAST_RECHECK_TICK, "token no longer valid")
        .await;
    match grpc_read(&mut alice_grpc, PAST_RECHECK_TICK).await {
        GrpcRead::Failed(status) => {
            assert_eq!(status.code(), tonic::Code::Unauthenticated, "{status}");
            assert!(
                status.message().contains("token no longer valid"),
                "{status}"
            );
        }
        GrpcRead::Event(event) => panic!("expected an UNAUTHENTICATED end, got {event:?}"),
        GrpcRead::Closed => panic!("expected an UNAUTHENTICATED end, got a clean close"),
        GrpcRead::Silent => panic!("the gRPC subscription outlived its credential"),
    }

    // Carol's and Bob's subscriptions, idle the same while, are still open.
    for stream in [&mut carol_rest, &mut bob_rest] {
        match stream.next(Duration::from_millis(200)).await {
            Frame::Silent => {}
            other => panic!("another member's stream should stay open, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn idle_streams_of_every_kind_end_within_the_recheck_tick() {
    let env = spawn().await;
    let a = env.workspace("Alpha").await;
    let b = env.workspace("Bravo").await;
    let (alice, alice_token) = env.member(a, "alice").await;
    let (_carol, carol_token) = env.member(a, "carol").await;
    let (_bob, bob_token) = env.member(b, "bob").await;

    let agui = format!("/agui/stream?workspace_id={}", a.0);
    let mut alice_streams = [
        ("/agui/stream", env.get_sse(&agui, &alice_token).await),
        (
            "/mcp/notifications",
            env.get_sse("/mcp/notifications", &alice_token).await,
        ),
        (
            "GET /mcp/streamable",
            env.get_sse("/mcp/streamable", &alice_token).await,
        ),
        (
            "POST /mcp/streamable",
            env.streamable_session(&alice_token).await,
        ),
    ];
    let mut carol_streams = [
        env.get_sse(&agui, &carol_token).await,
        env.get_sse("/mcp/notifications", &carol_token).await,
    ];
    let mut bob_streams = [
        env.get_sse(&format!("/agui/stream?workspace_id={}", b.0), &bob_token)
            .await,
        env.get_sse("/mcp/notifications", &bob_token).await,
    ];
    // The session stream opens with its `initialize` response.
    match alice_streams[3].1.next(Duration::from_secs(5)).await {
        Frame::Event(_, data) => assert!(data.contains("\"id\":1"), "{data}"),
        other => panic!("expected the initialize response, got {other:?}"),
    }

    env.deactivate(a, alice).await;

    // No frame is bound for these streams: each ends on the idle check.
    for (name, stream) in alice_streams.iter_mut() {
        eprintln!("checking {name}");
        stream
            .assert_ended(PAST_RECHECK_TICK, "token no longer valid")
            .await;
    }
    // Carol's and Bob's streams, idle the same while, are still open.
    for stream in carol_streams.iter_mut().chain(bob_streams.iter_mut()) {
        match stream.next(Duration::from_millis(200)).await {
            Frame::Silent => {}
            other => panic!("another member's stream should stay open, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_live_token_whose_scim_link_is_inactive_ends_too() {
    let env = spawn().await;
    let a = env.workspace("Alpha").await;
    let b = env.workspace("Bravo").await;
    let (alice, alice_token) = env.member(a, "alice").await;
    let (_carol, carol_token) = env.member(a, "carol").await;
    let (_bob, bob_token) = env.member(b, "bob").await;
    let mut alice_stream = env
        .get_sse(&format!("/mcp/stream?workspace_id={}", a.0), &alice_token)
        .await;
    let mut bob_stream = env
        .get_sse(&format!("/mcp/stream?workspace_id={}", b.0), &bob_token)
        .await;

    // Mark the SCIM link inactive without revoking the token, so only the
    // SCIM link check can end the stream.
    env.store
        .update_scim_user(alice, None, false)
        .await
        .unwrap()
        .expect("SCIM user");
    tokio::time::sleep(PAST_RECHECK_GAP).await;

    env.create_channel(a, &carol_token, "after-alice").await;
    alice_stream
        .assert_ended(Duration::from_secs(5), "member deactivated")
        .await;
    env.create_channel(b, &bob_token, "bravo-news").await;
    bob_stream.assert_carries("bravo-news").await;
}
