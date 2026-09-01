//! Unified delivery operator API (Cluster 80.0).

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations, AutomationDeliveryFilter};
use maidan_types::{AutomationSourceKind, NewAutomationDelivery, NewWebhookSubscription};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn unified_deliveries_list_and_replay_webhook_and_automation() {
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

    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "deliveries".into(),
        })
        .await
        .unwrap();

    let sub = store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws.id,
            url: "http://127.0.0.1:9/hook".into(),
            label: None,
            event_kinds: vec!["message.posted".into()],
            secret_ciphertext: "cipher".into(),
        })
        .await
        .unwrap();
    let webhook_id = store
        .enqueue_webhook_delivery(sub.id, 1, r#"{"kind":"message.posted"}"#)
        .await
        .unwrap();
    store.quarantine_webhook_delivery(webhook_id).await.unwrap();

    let auto_id = store
        .enqueue_automation_delivery(NewAutomationDelivery {
            workspace_id: ws.id,
            source_kind: AutomationSourceKind::SlashCommand,
            source_id: uuid::Uuid::new_v4(),
            target_url: "http://127.0.0.1:9/slash".into(),
            header_name: "X-Test".into(),
            header_value: "v".into(),
            payload: "{}".into(),
        })
        .await
        .unwrap();
    store.quarantine_automation_delivery(auto_id).await.unwrap();

    let store_for_assert = store.clone();
    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let all: serde_json::Value = client
        .get(format!("{base}/workspaces/{}/deliveries", ws.id.0))
        .query(&[("quarantined", "true")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let items = all.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|v| v["kind"] == "webhook"));
    assert!(items.iter().any(|v| v["kind"] == "automation"));

    let replay: serde_json::Value = client
        .post(format!(
            "{base}/workspaces/{}/deliveries/{webhook_id}/replay",
            ws.id.0
        ))
        .query(&[("kind", "webhook")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay["kind"], "webhook");
    assert!(replay["quarantined_at"].is_null());

    let pending = store_for_assert
        .list_webhook_deliveries(ws.id, AutomationDeliveryFilter::Pending, 10)
        .await
        .unwrap();
    assert!(pending.iter().any(|d| d.id == webhook_id));
}
