//! The A2A gRPC binding (§10). Spawns the tonic `lf.a2a.v1.A2AService` (the
//! official a2a.proto) against an in-memory store and drives every operation
//! with the generated gRPC client.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::a2a_grpc::generated::a2a_service_client::A2aServiceClient;
use maidan_server::a2a_grpc::generated::{
    part, send_message_response, stream_response, AuthenticationInfo, CancelTaskRequest,
    DeleteTaskPushNotificationConfigRequest, GetExtendedAgentCardRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTasksRequest, Message, Part, Role, SendMessageConfiguration, SendMessageRequest,
    SubscribeToTaskRequest, TaskPushNotificationConfig, TaskState,
};
use maidan_server::{AppState, FederationRuntime, WebhookRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations, A2aTaskWrite};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::atomic::AtomicI64;

/// A request naming A2A protocol version 1.0 in its metadata.
fn v1<T>(message: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("a2a-version", "1.0".parse().unwrap());
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn a2a_grpc_get_and_list_tasks() {
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
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let state = AppState::for_tests(store.clone(), artifacts, bus, search);

    // Seed a task directly in the store (workspace-mapped so RBAC resolves).
    let ws = store
        .create_workspace(NewWorkspace {
            name: "grpc".into(),
        })
        .await
        .unwrap();
    let task = serde_json::json!({
        "id": "task-1",
        "contextId": "ctx-1",
        "status": { "state": "TASK_STATE_WORKING" }
    });
    store
        .upsert_a2a_task(A2aTaskWrite {
            workspace_id: ws.id,
            task_id: "task-1",
            context_id: Some("ctx-1"),
            state: "TASK_STATE_WORKING",
            status_at: chrono::Utc::now(),
            task_json: task,
        })
        .await
        .unwrap();

    // Serve the gRPC binding on a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let svc = maidan_server::a2a_grpc::service(state);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .layer(maidan_server::trace_context::GrpcTraceLayer)
            .add_service(svc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    // Connect the generated client and exercise the ops.
    let mut client = A2aServiceClient::connect(format!("http://{addr}"))
        .await
        .expect("grpc connect");

    // A call that names no protocol version speaks 0.3: refused.
    let unversioned = client
        .get_task(GetTaskRequest {
            id: "task-1".into(),
            ..Default::default()
        })
        .await
        .expect_err("no a2a-version");
    assert_eq!(unversioned.code(), tonic::Code::Unimplemented);

    let got = client
        .get_task(v1(GetTaskRequest {
            id: "task-1".into(),
            ..Default::default()
        }))
        .await
        .expect("get_task")
        .into_inner();
    assert_eq!(got.id, "task-1");
    assert_eq!(got.status.unwrap().state(), TaskState::Working);

    // ListTasks routes over gRPC and returns the response shape. (Non-empty
    // contents under RBAC are proven by the auth-enabled test in channel_access_e2e;
    // this bypass server has no single workspace to scope the list by.)
    let listed = client
        .list_tasks(v1(ListTasksRequest::default()))
        .await
        .expect("list_tasks")
        .into_inner();
    assert!(listed.next_page_token.is_empty());

    // A missing task is NOT_FOUND (§10.6).
    let missing = client
        .get_task(v1(GetTaskRequest {
            id: "nope".into(),
            ..Default::default()
        }))
        .await
        .expect_err("missing task");
    assert_eq!(missing.code(), tonic::Code::NotFound);
    // It carries the A2A ErrorInfo in the status details, as on the other
    // bindings: google.rpc.Status { details: [Any(ErrorInfo)] }.
    let details = missing.details();
    let as_text = String::from_utf8_lossy(details);
    assert!(
        as_text.contains("type.googleapis.com/google.rpc.ErrorInfo")
            && as_text.contains("TASK_NOT_FOUND")
            && as_text.contains("a2a-protocol.org"),
        "status details: {as_text:?}"
    );

    // Cancel ends a working task; a second cancel is FAILED_PRECONDITION.
    let canceled = client
        .cancel_task(v1(CancelTaskRequest {
            id: "task-1".into(),
            ..Default::default()
        }))
        .await
        .expect("cancel_task")
        .into_inner();
    assert_eq!(canceled.status.unwrap().state(), TaskState::Canceled);
    let again = client
        .cancel_task(v1(CancelTaskRequest {
            id: "task-1".into(),
            ..Default::default()
        }))
        .await
        .expect_err("already canceled");
    assert_eq!(again.code(), tonic::Code::FailedPrecondition);
}

/// A request as a member: version 1.0 and a bearer token.
fn authed<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = v1(message);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    request
}

fn text_message(text: &str) -> Message {
    Message {
        message_id: uuid::Uuid::new_v4().to_string(),
        role: Role::User.into(),
        parts: vec![Part {
            content: Some(part::Content::Text(text.into())),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Every operation of the official service over gRPC, against an auth-enabled
/// server: the same operations the JSON-RPC and REST bindings run.
#[tokio::test(flavor = "multi_thread")]
async fn a2a_grpc_serves_the_whole_service() {
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

    let ws = store
        .create_workspace(NewWorkspace {
            name: "grpc".into(),
        })
        .await
        .unwrap()
        .id;
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "caller".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id;
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: [
                capability::WORKSPACE_READ,
                capability::WORKSPACE_WRITE,
                capability::MESSAGE_POST,
            ]
            .iter()
            .map(|c| c.to_string())
            .collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    let token = secret.as_str().to_string();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let svc = maidan_server::a2a_grpc::service(state);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .layer(maidan_server::trace_context::GrpcTraceLayer)
            .add_service(svc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let mut client = A2aServiceClient::connect(format!("http://{addr}"))
        .await
        .expect("grpc connect");

    // No token: UNAUTHENTICATED.
    let anonymous = client
        .get_task(v1(GetTaskRequest {
            id: "x".into(),
            ..Default::default()
        }))
        .await
        .expect_err("no bearer");
    assert_eq!(anonymous.code(), tonic::Code::Unauthenticated);

    // SendMessage: a completed task whose history carries the message.
    let sent = client
        .send_message(authed(
            SendMessageRequest {
                message: Some(text_message("hello over grpc")),
                configuration: Some(SendMessageConfiguration {
                    history_length: Some(10),
                    ..Default::default()
                }),
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect("send_message")
        .into_inner();
    let Some(send_message_response::Payload::Task(task)) = sent.payload else {
        panic!("SendMessage returns a task: {sent:?}");
    };
    assert_eq!(task.status.as_ref().unwrap().state(), TaskState::Completed);
    assert!(!task.context_id.is_empty());
    assert!(task.status.as_ref().unwrap().timestamp.is_some());
    let words: Vec<_> = task
        .history
        .iter()
        .flat_map(|m| &m.parts)
        .filter_map(|p| match &p.content {
            Some(part::Content::Text(t)) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert!(words.contains(&"hello over grpc"), "history: {words:?}");

    // The gRPC server has no request layer, so the binding scopes each call
    // as its caller: the message it posted is the caller's, not the system's.
    let last = store.max_event_id().await.unwrap();
    let mut attributed = false;
    for id in 1..=last {
        let event = store.get_stored_event(id).await.unwrap();
        attributed |= event.attribution().is_some_and(|a| a.actor_id == member);
    }
    assert!(
        attributed,
        "SendMessage over gRPC wrote no event attributed to its caller"
    );

    // A message with a part that has no content is the caller's error.
    let mut empty = text_message("x");
    empty.parts[0].content = None;
    let refused = client
        .send_message(authed(
            SendMessageRequest {
                message: Some(empty),
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect_err("empty part");
    assert_eq!(refused.code(), tonic::Code::InvalidArgument);

    // SendStreamingMessage: the working task, then its completion.
    let mut stream = client
        .send_streaming_message(authed(
            SendMessageRequest {
                message: Some(text_message("streamed")),
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect("send_streaming_message")
        .into_inner();
    let mut kinds = Vec::new();
    while let Some(event) = stream.message().await.expect("stream event") {
        kinds.push(match event.payload.expect("payload") {
            stream_response::Payload::Task(t) => {
                assert_eq!(t.status.unwrap().state(), TaskState::Working);
                "task"
            }
            stream_response::Payload::StatusUpdate(u) => {
                assert_eq!(u.status.unwrap().state(), TaskState::Completed);
                "status"
            }
            other => panic!("unexpected event {other:?}"),
        });
    }
    assert_eq!(kinds, ["task", "status"]);

    // GetTask and ListTasks see the sent task.
    let got = client
        .get_task(authed(
            GetTaskRequest {
                id: task.id.clone(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect("get_task")
        .into_inner();
    assert_eq!(got.id, task.id);
    let listed = client
        .list_tasks(authed(
            ListTasksRequest {
                context_id: task.context_id.clone(),
                page_size: Some(10),
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect("list_tasks")
        .into_inner();
    assert_eq!(listed.total_size, 1);
    assert_eq!(listed.tasks[0].id, task.id);
    assert!(listed.next_page_token.is_empty());

    // A finished task has nothing to subscribe to: UNIMPLEMENTED
    // (UnsupportedOperation), as on the other bindings.
    let subscribed = client
        .subscribe_to_task(authed(
            SubscribeToTaskRequest {
                id: task.id.clone(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect_err("a completed task streams nothing");
    assert_eq!(subscribed.code(), tonic::Code::Unimplemented);

    // Push configs: create (secrets accepted, never echoed), get, page, delete.
    for id in ["a", "b"] {
        let created = client
            .create_task_push_notification_config(authed(
                TaskPushNotificationConfig {
                    id: id.into(),
                    task_id: task.id.clone(),
                    url: "https://hooks.example.com/a2a".into(),
                    token: "per-task-token".into(),
                    authentication: Some(AuthenticationInfo {
                        scheme: "Bearer".into(),
                        credentials: "secret".into(),
                    }),
                    ..Default::default()
                },
                &token,
            ))
            .await
            .expect("create push config")
            .into_inner();
        assert_eq!(created.id, id);
        assert_eq!(created.task_id, task.id);
    }
    // A config writes no event, so the call is recorded under its method.
    let recorded = store.list_audit(100).await.unwrap();
    let created_rows = recorded
        .iter()
        .filter(|row| {
            row.action == maidan_server::auth::MUTATION_ACTION
                && row.actor_id == Some(member)
                && row.metadata["binding"] == "grpc"
                && row.metadata["operation"] == "CreateTaskPushNotificationConfig"
        })
        .count();
    assert_eq!(created_rows, 2, "{recorded:?}");
    assert!(
        recorded
            .iter()
            .filter(|row| row.action == maidan_server::auth::MUTATION_ACTION)
            .all(|row| maidan_a2a::Method::ALL
                .iter()
                .any(|m| m.changes() && row.metadata["operation"] == m.name())),
        "a read over gRPC was recorded as a change: {recorded:?}"
    );
    let got = client
        .get_task_push_notification_config(authed(
            GetTaskPushNotificationConfigRequest {
                task_id: task.id.clone(),
                id: "a".into(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect("get push config")
        .into_inner();
    assert_eq!(got.url, "https://hooks.example.com/a2a");
    let page = client
        .list_task_push_notification_configs(authed(
            ListTaskPushNotificationConfigsRequest {
                task_id: task.id.clone(),
                page_size: 1,
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect("list push configs")
        .into_inner();
    assert_eq!(page.configs.len(), 1);
    assert!(!page.next_page_token.is_empty());
    client
        .delete_task_push_notification_config(authed(
            DeleteTaskPushNotificationConfigRequest {
                task_id: task.id.clone(),
                id: "a".into(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect("delete push config");
    let gone = client
        .get_task_push_notification_config(authed(
            GetTaskPushNotificationConfigRequest {
                task_id: task.id.clone(),
                id: "a".into(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .expect_err("deleted");
    assert_eq!(gone.code(), tonic::Code::NotFound);

    // The extended card, as the proto's AgentCard.
    let card = client
        .get_extended_agent_card(authed(GetExtendedAgentCardRequest::default(), &token))
        .await
        .expect("extended card")
        .into_inner();
    assert!(!card.name.is_empty());
    assert!(!card.supported_interfaces.is_empty());
}
