//! Overload, panics and credentials in traces, through the real router.
//!
//! - Past the in-flight ceiling a request is refused at once with a `503`
//!   problem and `Retry-After`, while health probes and `/metrics` still
//!   answer, and a finished request gives its permit back.
//! - A handler that panics answers a `500` problem carrying `X-Request-Id`;
//!   the connection is not dropped and the server keeps serving.
//!
//! Header redaction in traces is `trace_redaction_e2e`.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_search::{EmbeddingProvider, EmbeddingProviderError};
use maidan_server::{
    load_shed::{Admission, RequestLimit},
    router, AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::NewWorkspace;
use reqwest::StatusCode;
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

async fn state() -> (AppState, tempfile::TempDir) {
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
    let state = AppState::for_tests(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
    );
    (state, dir)
}

async fn serve(state: AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_request_past_the_in_flight_ceiling_is_shed_with_a_503_problem() {
    let (mut state, _dir) = state().await;
    let limit = RequestLimit::new(1);
    state.request_limit = limit.clone();
    let base = serve(state).await;
    let client = client();

    // Another request holds the only permit.
    let Admission::Admitted(held) = limit.admit() else {
        panic!("the permit must be free");
    };

    let shed = client
        .get(format!("{base}/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(shed.headers()["retry-after"], "1");
    assert_eq!(shed.headers()["content-type"], "application/problem+json");
    assert!(shed.headers().contains_key("x-request-id"));
    let problem: Value = shed.json().await.unwrap();
    assert_eq!(problem["type"], "https://maidan.dev/problems/overloaded");
    assert_eq!(problem["status"], 503);

    // Probes and the scrape are never shed.
    for path in ["/health/live", "/metrics"] {
        let resp = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} was shed");
    }

    drop(held);
    let served = client
        .get(format!("{base}/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(served.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_finished_request_gives_back_its_permit() {
    let (mut state, _dir) = state().await;
    let limit = RequestLimit::new(1);
    state.request_limit = limit.clone();
    let base = serve(state).await;
    let client = client();

    // With one permit, each of these can run only if the one before it gave
    // its permit back.
    for _ in 0..5 {
        let resp = client
            .get(format!("{base}/openapi.json"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
    // An error response gives it back too.
    let missing = client.get(format!("{base}/nope")).send().await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(limit.in_flight(), 0);
}

/// An embedding provider that panics, standing in for any handler bug.
struct PanickingProvider;

impl EmbeddingProvider for PanickingProvider {
    fn model_name(&self) -> &str {
        "panicking"
    }
    fn dimension(&self) -> usize {
        1
    }
    fn embed(&self, _body: &str) -> Result<Vec<f32>, EmbeddingProviderError> {
        panic!("embedding provider exploded with internal detail");
    }
}

#[tokio::test]
async fn a_panicking_handler_answers_a_500_problem_and_the_server_keeps_serving() {
    let (mut state, _dir) = state().await;
    let ws = state
        .store
        .create_workspace(NewWorkspace {
            name: "panics".into(),
        })
        .await
        .unwrap();
    state.embedding_provider = Arc::new(PanickingProvider);
    let limit = state.request_limit.clone();
    let base = serve(state).await;
    let client = client();

    let resp = client
        .get(format!("{base}/workspaces/{}/search", ws.id.0))
        .query(&[("q", "anything"), ("mode", "semantic")])
        // CI runs with RUST_BACKTRACE=1, and the panic hook symbolizes the
        // backtrace before the 500 goes out, which can outlast the client's
        // 5 s on a loaded runner. This asserts an answer, not its latency.
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .expect("a panic must be answered, not a dropped connection");
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(resp.headers()["content-type"], "application/problem+json");
    assert!(
        resp.headers().contains_key("x-request-id"),
        "the 500 must carry the request id the log is keyed by"
    );
    let text = resp.text().await.unwrap();
    assert!(
        !text.contains("internal detail"),
        "the panic message must not reach the client: {text}"
    );
    let problem: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(problem["type"], "https://maidan.dev/problems/internal");
    assert_eq!(problem["detail"], maidan_server::panic_guard::PANIC_DETAIL);

    // The panicking request released its permit and the server still serves.
    assert_eq!(limit.in_flight(), 0);
    let lexical = client
        .get(format!("{base}/workspaces/{}/search", ws.id.0))
        .query(&[("q", "anything")])
        .send()
        .await
        .unwrap();
    assert_eq!(lexical.status(), StatusCode::OK);
}
