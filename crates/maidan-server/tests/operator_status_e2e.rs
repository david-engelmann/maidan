//! `GET /operator/status`: an operator reads phase, search backfill and queue
//! depths in one place, as JSON or as a page. It spans every workspace, so a
//! workspace admin cannot read it.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn token(store: &dyn Store, capabilities: &[&str]) -> String {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    format!("Bearer {}", secret.as_str())
}

#[tokio::test]
async fn operator_status_reports_phase_backfill_and_queues() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let draining = state.draining.clone();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let url = format!("http://{addr}/operator/status");

    let operator = token(store.as_ref(), &["operator:global"]).await;
    let workspace_admin = token(store.as_ref(), &["token:admin", "workspace:read"]).await;

    let refused = client
        .get(&url)
        .header("authorization", &workspace_admin)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    // Give the log a head the tap has not reached.
    for name in ["a", "b"] {
        store
            .create_workspace_with_event(NewWorkspace { name: name.into() })
            .await
            .unwrap();
    }
    let head = store.max_event_id().await.unwrap();
    assert!(head > 0);
    store
        .set_tap_cursor(maidan_search::SEARCH_TAP_SURFACE, head / 2)
        .await
        .unwrap();

    let body: serde_json::Value = client
        .get(&url)
        .header("authorization", &operator)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["phase"], "serving");
    assert_eq!(body["search"]["event_log_head"], head);
    assert_eq!(body["search"]["tap_cursor"], head / 2);
    assert_eq!(body["search"]["behind"], head - head / 2);
    assert!(body["search"]["backfill_percent"].as_f64().unwrap() < 100.0);
    assert!(body["replica"].is_null(), "no replica configured");
    assert!(body["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == "db" && c["ok"] == true));

    draining.store(true, std::sync::atomic::Ordering::Relaxed);
    let page = client
        .get(&url)
        .header("authorization", &operator)
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert!(page
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let html = page.text().await.unwrap();
    assert!(html.contains("<th>Phase</th><td>draining</td>"), "{html}");
    assert!(html.contains("Search backfill"));

    server.abort();
}
