//! A queued webhook delivery goes out only for an event in its hook's own
//! workspace. Enqueue already matches hooks by the event's workspace; this
//! pins the retry path, which reads the event back by log id and so must
//! check the workspace itself: a workspace-level event (no channel or
//! thread) passes the visibility check for any reader.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{test_support, webhook_worker, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;

async fn state() -> (AppState, tempfile::TempDir) {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::for_tests(
        Arc::new(SqliteStore::for_tests(pool.clone())),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
    );
    (state, dir)
}

/// The log id of a workspace-level event in `ws`.
async fn workspace_event(store: &dyn Store, ws: WorkspaceId, name: &str) -> i64 {
    let (_, stored) = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws,
            name: name.into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    stored.id
}

#[tokio::test]
async fn a_queued_delivery_carrying_another_workspaces_event_is_quarantined_unsent() {
    let (state, _dir) = state().await;
    let store = state.store.as_ref();
    let a = test_support::tenant(store, "a").await.workspace_id;
    let b = test_support::tenant(store, "b").await.workspace_id;
    let creator = test_support::registrar(store, a).await;
    let sub = store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: a,
            // Nothing listens here: a delivery that is attempted records an
            // attempt, one that is refused first records none.
            url: "http://127.0.0.1:9/hook".into(),
            label: None,
            event_kinds: vec!["channel_created".into()],
            secret_ciphertext: "x".into(),
            created_by_token: Some(creator),
        })
        .await
        .unwrap();
    maidan_server::webhooks::remember_webhook_secret(&state.webhooks.secrets, sub.id, "s".into());

    let own = store
        .enqueue_webhook_delivery(sub.id, workspace_event(store, a, "own").await, "{}")
        .await
        .unwrap()
        .unwrap();
    let foreign = store
        .enqueue_webhook_delivery(sub.id, workspace_event(store, b, "foreign").await, "{}")
        .await
        .unwrap()
        .unwrap();

    webhook_worker::poll_deliveries(&state, 16).await.unwrap();

    let own = store.get_webhook_delivery(own, a).await.unwrap();
    let foreign = store.get_webhook_delivery(foreign, a).await.unwrap();
    assert!(
        own.quarantined_at.is_none() && (own.attempts > 0 || own.delivered_at.is_some()),
        "the hook's own workspace event is attempted: {own:?}"
    );
    assert!(
        foreign.quarantined_at.is_some(),
        "another workspace's event is quarantined: {foreign:?}"
    );
    assert_eq!(foreign.attempts, 0, "and never sent: {foreign:?}");
    assert!(foreign.delivered_at.is_none());
}
