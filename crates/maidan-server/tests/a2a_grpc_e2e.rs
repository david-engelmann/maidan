//! Cluster 287: the A2A gRPC binding (§10). Spawns the tonic A2AService against
//! an in-memory store and drives it with the generated gRPC client.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::a2a_grpc::generated::a2a_service_client::A2aServiceClient;
use maidan_server::a2a_grpc::generated::{GetTaskRequest, ListTasksRequest};
use maidan_server::AppState;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::NewWorkspace;
use sqlx::sqlite::SqlitePoolOptions;

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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
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
    store.upsert_a2a_task(ws.id, "task-1", task).await.unwrap();

    // Serve the gRPC binding on a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let svc = maidan_server::a2a_grpc::service(state);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(svc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    // Connect the generated client and exercise the ops.
    let mut client = A2aServiceClient::connect(format!("http://{addr}"))
        .await
        .expect("grpc connect");

    let got = client
        .get_task(GetTaskRequest {
            id: "task-1".into(),
        })
        .await
        .expect("get_task")
        .into_inner();
    assert_eq!(got.id, "task-1");
    assert_eq!(got.status.unwrap().state, "TASK_STATE_WORKING");

    // ListTasks routes over gRPC and returns the response shape. (Non-empty
    // contents under RBAC are proven by the auth-enabled test in channel_access_e2e;
    // this bypass server has no single workspace to scope the list by.)
    let listed = client
        .list_tasks(ListTasksRequest {
            context_id: String::new(),
            page_size: 0,
        })
        .await
        .expect("list_tasks")
        .into_inner();
    assert!(listed.next_page_token.is_empty());

    // A missing task maps to a gRPC error status, not a panic.
    let missing = client.get_task(GetTaskRequest { id: "nope".into() }).await;
    assert!(missing.is_err(), "missing task should be a gRPC error");
}
