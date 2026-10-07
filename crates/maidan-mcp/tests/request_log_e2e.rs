//! The MCP request log reaches a real subscriber at the default filter: one
//! `info` event per request on `maidan_mcp::request` with the tool and the
//! argument names, and never an argument value.

use std::io::Write;
use std::sync::{Arc, Mutex};

use maidan_artifacts::LocalFsStore;
use maidan_auth::AuthContext;
use maidan_mcp::{JsonRpcRequest, McpServer};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tracing_subscriber::fmt::MakeWriter;

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

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

async fn mk_server() -> McpServer {
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
    McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn every_request_logs_its_tool_and_argument_names_and_never_a_value() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter("info,sqlx=warn")
        .with_writer(captured.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let server = mk_server().await;
    let auth = AuthContext::bypass();
    let call = |name: &str, arguments: serde_json::Value| JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/call".into(),
        params: json!({ "name": name, "arguments": arguments }),
    };
    server
        .handle(
            call(
                "post_message",
                json!({
                    "thread_id": "thread-marker-qqq",
                    "body": "the launch code is 0000"
                }),
            ),
            &auth,
        )
        .await;
    server
        .handle(call("sk-live-not-a-tool", json!({})), &auth)
        .await;

    let text = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| l.contains("mcp request")).collect();
    assert_eq!(lines.len(), 2, "one line per request: {text}");
    assert!(lines[0].contains("maidan_mcp::request"), "{}", lines[0]);
    assert!(lines[0].contains("tool=post_message"), "{}", lines[0]);
    assert!(lines[0].contains("arg_keys=body,thread_id"), "{}", lines[0]);
    assert!(lines[0].contains("transport=\"http\""), "{}", lines[0]);
    assert!(lines[0].contains("outcome="), "{}", lines[0]);
    assert!(lines[0].contains("latency_ms="), "{}", lines[0]);
    assert!(lines[1].contains("tool=(unknown)"), "{}", lines[1]);
    for leaked in ["launch code", "qqq", "sk-live"] {
        assert!(!text.contains(leaked), "{leaked} reached the log: {text}");
    }
}
