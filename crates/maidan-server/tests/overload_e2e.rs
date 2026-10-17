//! Overload, panics and credentials in traces, through the real router.
//!
//! - Past the in-flight ceiling a request is refused at once with a `503`
//!   problem and `Retry-After`, while health probes and `/metrics` still
//!   answer, and a finished request gives its permit back.
//! - A handler that panics answers a `500` problem carrying `X-Request-Id`;
//!   the connection is not dropped and the server keeps serving.
//! - A slow embedding endpoint holds only the search that is waiting on it;
//!   an unrelated request on the same runtime is still answered.
//!
//! Header redaction in traces is `trace_redaction_e2e`.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

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

/// An OpenAI-compatible endpoint that answers its one request only once
/// `release` fires, or after `SLOW_ENDPOINT_FALLBACK` if it never does. It
/// reports arrival on `arrived` and whether `release` came in time in the
/// returned flag.
const SLOW_ENDPOINT_FALLBACK: Duration = Duration::from_secs(10);

fn slow_endpoint(
    arrived: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
) -> (String, Arc<AtomicBool>) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let released = Arc::new(AtomicBool::new(false));
    let flag = released.clone();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let _ = arrived.send(());
        flag.store(
            release.recv_timeout(SLOW_ENDPOINT_FALLBACK).is_ok(),
            Ordering::SeqCst,
        );
        let reply = r#"{"data":[{"index":0,"embedding":[0.1,0.2,0.3]}]}"#;
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        );
    });
    (format!("http://{addr}/v1/embeddings"), released)
}

// A current-thread runtime has one worker, so an embed that blocks it stops
// everything else. The endpoint holds its answer until the unrelated request
// has been served; if the embed held the worker, that request cannot be served
// until the endpoint gives up waiting, and `released` stays false.
#[tokio::test]
async fn a_slow_embedding_endpoint_does_not_stall_an_unrelated_request() {
    let (mut state, _dir) = state().await;
    let ws = state
        .store
        .create_workspace(NewWorkspace {
            name: "slow-embeddings".into(),
        })
        .await
        .unwrap();
    let (arrived_tx, arrived_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (endpoint, released) = slow_endpoint(arrived_tx, release_rx);
    state.embedding_provider = Arc::new(
        maidan_search::embedding_provider::OpenAiCompatibleProvider::new(
            endpoint,
            "slow-stub".into(),
            None,
            Some(3),
            30,
        )
        .unwrap(),
    );
    let base = serve(state).await;
    let client = client();

    let search = tokio::spawn({
        let client = client.clone();
        let url = format!("{base}/workspaces/{}/search", ws.id.0);
        async move {
            client
                .get(url)
                .query(&[("q", "anything"), ("mode", "semantic")])
                .timeout(Duration::from_secs(30))
                .send()
                .await
        }
    });
    arrived_rx.await.unwrap();

    let health = client
        .get(format!("{base}/health/live"))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let _ = release_tx.send(());

    search
        .await
        .unwrap()
        .expect("the search is answered once the endpoint is");
    assert!(
        released.load(Ordering::SeqCst),
        "the unrelated request waited for the embedding endpoint"
    );
}
