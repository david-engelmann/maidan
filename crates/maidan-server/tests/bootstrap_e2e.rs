//! Bootstrap route gating when bearer auth is enabled.
#![cfg(feature = "bootstrap")]

use std::path::PathBuf;
use std::process::Command;
use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

async fn spawn(bootstrap_enabled: bool) -> Harness {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        bootstrap_enabled,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    Harness {
        addr,
        server,
        client,
        _dir: dir,
    }
}

#[tokio::test]
async fn bootstrap_routes_reject_when_flag_unset_and_auth_enabled() {
    let h = spawn(false).await;
    let base = h.base();
    let res = h
        .client
        .post(format!("{base}/workspaces"))
        .json(&json!({ "name": "blocked" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    h.shutdown().await;
}

#[tokio::test]
async fn bootstrap_creates_workspace_and_member_when_flag_set() {
    let h = spawn(true).await;
    let base = h.base();
    let ws: serde_json::Value = h
        .client
        .post(format!("{base}/workspaces"))
        .json(&json!({ "name": "seed" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();
    let member = h
        .client
        .post(format!("{base}/workspaces/{wid}/members"))
        .json(&json!({
            "handle": "admin",
            "kind": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(member.status(), StatusCode::CREATED);
    h.shutdown().await;
}

#[tokio::test]
async fn bootstrap_rejects_second_workspace_creation() {
    let h = spawn(true).await;
    let base = h.base();
    let first = h
        .client
        .post(format!("{base}/workspaces"))
        .json(&json!({ "name": "one" }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let second = h
        .client
        .post(format!("{base}/workspaces"))
        .json(&json!({ "name": "two" }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::FORBIDDEN);
    h.shutdown().await;
}

/// scripts/demo-board.sh against this server: it creates the cast, plays the
/// story, the coder's close is refused, David's inbox is the one login task,
/// the human closes it, and the hash chain verifies.
#[tokio::test]
async fn demo_board_script_hits_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("demo.sqlite");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    // One connection, and a busy timeout, matching the server: two agents
    // claim at the same moment, and a multi-connection SQLite pool turns
    // that into "database is locked" instead of waiting.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("PRAGMA busy_timeout = 5000")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&db_url)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "demo".into(),
        })
        .await
        .unwrap();
    let admin = store
        .create_member(maidan_types::NewMember {
            workspace_id: ws.id,
            handle: "admin".into(),
            display_name: Some("Admin".into()),
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = maidan_auth::TokenSecret::generate();
    store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id: ws.id,
            member_id: admin.id,
            app_installation_id: None,
            token_hash: maidan_auth::hash_secret(secret.as_str()),
            label: Some("admin".into()),
            capabilities: maidan_auth::capability::all(),
            expires_at: None,
        })
        .await
        .unwrap();
    let artifacts = dir.path().join("artifacts");
    std::fs::create_dir_all(&artifacts).unwrap();
    let app = router(AppState::new(
        store,
        Arc::new(maidan_artifacts::LocalFsStore::new(&artifacts)),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/demo-board.sh");
    let secret_str = secret.as_str().to_string();
    let ws_id = ws.id.0.to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new("bash")
            .arg(&script)
            .env("MAIDAN_URL", format!("http://{addr}"))
            .env("MAIDAN_TOKEN", secret_str)
            .env("MAIDAN_WORKSPACE", ws_id)
            .env("DEMO_PAUSE", "0")
            .output()
            .expect("run demo-board.sh")
    })
    .await
    .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "demo-board.sh failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("review requirement not met"), "{stdout}");
    assert!(
        stdout.contains("Needs you: 1 · Fix the flaky login test"),
        "{stdout}"
    );
    assert!(stdout.contains("ok=true"), "{stdout}");
    assert!(
        stdout.contains("closed: Fix the flaky login test"),
        "{stdout}"
    );
    server.abort();
}
