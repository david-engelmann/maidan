//! The request span shows that `Authorization`, `Cookie` and `Mcp-Session-Id`
//! were sent, never their values, and leaves out the query string.
//!
//! Alone in its binary: it installs a thread-local subscriber, and tracing
//! caches a callsite's interest globally, so a test running beside it on
//! another thread can register a callsite as uninteresting first.

use std::{
    io::Write,
    sync::{Arc, Mutex},
};

use axum::{body::Body, http::Request};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tower::ServiceExt;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn trace_spans_show_credential_headers_as_sensitive_and_leave_out_the_query() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(captured.clone())
        .finish();
    // The router runs on this thread (`oneshot`, current-thread runtime), so
    // a thread-local subscriber sees every span it makes.
    let _guard = tracing::subscriber::set_default(subscriber);

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
    let app = router(AppState::for_tests(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
    ));

    let unknown = app
        .clone()
        .oneshot(
            Request::get("/nope?code=one-time-oauth-code")
                .header("authorization", "Bearer mdn_bearer_secret_value")
                .header("cookie", "maidan_session=session_cookie_secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);

    let init = app
        .oneshot(
            Request::post("/mcp/streamable")
                .header("content-type", "application/json")
                .header("mcp-protocol-version", "2024-11-05")
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(init.status(), 200);
    let session = init.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_string();

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("path=/nope"), "no request span in: {log}");
    assert!(log.contains("\"authorization\": Sensitive"), "{log}");
    assert!(log.contains("\"cookie\": Sensitive"), "{log}");
    assert!(log.contains("\"mcp-session-id\": Sensitive"), "{log}");
    for secret in [
        "mdn_bearer_secret_value",
        "session_cookie_secret",
        "one-time-oauth-code",
        session.as_str(),
    ] {
        assert!(
            !log.contains(secret),
            "{secret} leaked into the trace: {log}"
        );
    }
}
