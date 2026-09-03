//! Seed-and-serve harness for the Playwright `/ui` suite (Cluster 350.7).
//!
//! This is **test support, not a shipped binary**: it stands up the real
//! `maidan-server` router on an in-memory SQLite store, seeds a deterministic
//! workspace / channel / thread / pending approval gate, mints a bearer token,
//! writes the fixtures to a JSON file, and then serves forever so a headless
//! browser can drive the actual `/ui`. Playwright's `webServer` starts it, waits
//! for `/ui/`, runs the specs, and kills it.
//!
//! Env: `UI_TEST_PORT` (default 8899), `UI_TEST_FIXTURES` (default
//! `ui-tests/.fixtures.json`). Run via `cargo run --example ui_test_server`.

use std::net::SocketAddr;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewApprovalGate, NewChannel, NewMember, NewThread, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("UI_TEST_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8899);
    let fixtures_path =
        std::env::var("UI_TEST_FIXTURES").unwrap_or_else(|_| "ui-tests/.fixtures.json".into());

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));

    // Deterministic seed the specs assert against.
    let ws = store
        .create_workspace(NewWorkspace {
            name: "UI Test Workspace".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "operator".into(),
            display_name: Some("Operator".into()),
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("Deploy v9".into()),
        })
        .await
        .expect("thread");
    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: member.id,
            prompt: "Deploy v9 to prod?".into(),
            schema: None,
        })
        .await
        .expect("gate");

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("ui-test".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .expect("token");

    let art_dir = std::env::temp_dir().join(format!("maidan-ui-test-{}", std::process::id()));
    std::fs::create_dir_all(&art_dir).expect("art dir");
    let artifacts = Arc::new(LocalFsStore::new(&art_dir));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED — the specs drive the real bearer path
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    // The approval-gate `request_state` HMAC + subscribe-resume are secret-keyed.
    state.subscribe_resume_secret = Some(Arc::from(&b"ui-test-subscribe-resume-secret-32b"[..]));

    let fixtures = serde_json::json!({
        "base_url": format!("http://127.0.0.1:{port}"),
        "token": secret.as_str(),
        "workspace_id": ws.id.0.to_string(),
        "member_id": member.id.0.to_string(),
        "channel_id": channel.id.0.to_string(),
        "thread_id": thread.id.0.to_string(),
        "gate_id": gate.id.0.to_string(),
    });
    std::fs::write(
        &fixtures_path,
        serde_json::to_string_pretty(&fixtures).expect("fixtures json"),
    )
    .expect("write fixtures");

    let app = router(state);
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind (is UI_TEST_PORT free?)");
    eprintln!("ui_test_server: seeded + listening on http://{addr} (fixtures: {fixtures_path})");
    axum::serve(listener, app).await.expect("serve");
}
