//! Cluster 399.2: a WASI slash handler runs end to end, and only over a module
//! its own workspace owns.
//!
//! Before this, `SlashHandlerKind::wasi` was registrable and every dispatch
//! returned `wasi_runtime_unavailable` — a user could configure a handler that
//! could never run.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime, SlashRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

fn test_key() -> Option<Arc<[u8; 32]>> {
    Some(Arc::new([7u8; 32]))
}

/// A guest that echoes a fixed string on stdout and exits cleanly.
fn echo_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
             (import "wasi_snapshot_preview1" "fd_write"
               (func $fd_write (param i32 i32 i32 i32) (result i32)))
             (memory (export "memory") 1)
             (data (i32.const 8) "handled by wasm")
             (func (export "_start")
               (i32.store (i32.const 0) (i32.const 8))
               (i32.store (i32.const 4) (i32.const 15))
               (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 40)))))"#,
    )
    .expect("valid wat")
}

/// A guest that reaches for the filesystem — must never run.
fn escaping_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
             (import "wasi_snapshot_preview1" "path_open"
               (func $open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
             (func (export "_start")))"#,
    )
    .expect("valid wat")
}

struct Ctx {
    base: String,
    client: reqwest::Client,
    ws: maidan_types::WorkspaceId,
    thread: maidan_types::ThreadId,
    author: maidan_types::MemberId,
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
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        true,
        FederationRuntime::new(true, test_key()),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.slash = SlashRuntime::new(test_key());
    std::mem::forget(dir);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();

    Ctx {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        ws: ws.id,
        thread: thread.id,
        author: author.id,
    }
}

impl Ctx {
    async fn upload(&self, bytes: Vec<u8>) -> String {
        let resp = self
            .client
            .post(format!("{}/artifacts?kind=attachment", self.base))
            .header("content-type", "application/wasm")
            .body(bytes)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::CREATED, "upload failed");
        let v: Value = resp.json().await.unwrap();
        v["sha256"]
            .as_str()
            .expect("sha256 in response")
            .to_string()
    }

    async fn register(&self, name: &str, target: &str) {
        let resp = self
            .client
            .post(format!(
                "{}/workspaces/{}/slash-commands",
                self.base, self.ws.0
            ))
            .json(&json!({
                "name": name,
                "handler_kind": "wasi",
                "handler_target": target
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::CREATED,
            "register failed"
        );
    }

    async fn invoke(&self, body: &str) -> Value {
        self.client
            .post(format!("{}/threads/{}/messages", self.base, self.thread.0))
            .json(&json!({ "author_id": self.author.0, "body": body }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

/// The headline: registering a wasm module and typing the command runs it.
#[tokio::test]
async fn a_wasi_slash_command_runs_its_module() {
    let ctx = spawn().await;
    let sha = ctx.upload(echo_module()).await;
    ctx.register("wasm", &sha).await;

    let msg = ctx.invoke("/wasm hello").await;
    let response = &msg["metadata"]["slash_response"];
    assert_eq!(
        response["ok"], true,
        "handler should have run, got {response:?}"
    );
    assert_eq!(
        response["response"]["content"][0]["text"], "handled by wasm",
        "the guest's stdout is the slash response"
    );
}

/// A sha with no bytes behind it fails cleanly rather than running anything.
///
/// The *tenancy* half of this — a well-formed sha belonging to another
/// workspace — is covered in `wasi_tenancy_e2e`, which runs with auth enabled;
/// under `AUTH_DISABLED` the upload path records no access links at all, so
/// there is no ownership to test here.
#[tokio::test]
async fn a_sha_with_no_module_behind_it_is_refused() {
    let ctx = spawn().await;
    let orphan = "a".repeat(64);
    ctx.register("orphan", &orphan).await;

    let msg = ctx.invoke("/orphan").await;
    let response = &msg["metadata"]["slash_response"];
    assert_eq!(response["ok"], false);
    assert_eq!(
        response["error_kind"], "invalid_module",
        "a missing module is refused, not executed: {response:?}"
    );
}

/// The sandbox still applies through the slash path — a filesystem import is
/// refused as a banned import, not run and not silently ignored.
#[tokio::test]
async fn a_module_reaching_for_the_filesystem_is_refused_through_the_slash_path() {
    let ctx = spawn().await;
    let sha = ctx.upload(escaping_module()).await;
    ctx.register("escape", &sha).await;

    let msg = ctx.invoke("/escape").await;
    let response = &msg["metadata"]["slash_response"];
    assert_eq!(response["ok"], false);
    assert_eq!(
        response["error_kind"], "banned_import",
        "path_open must be refused at the slash surface too: {response:?}"
    );
    let err = response["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("path_open"),
        "the error should name the import, got: {err}"
    );
}
