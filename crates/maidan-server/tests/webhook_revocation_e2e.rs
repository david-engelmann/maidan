//! The webhook poller stops at a revoked hook and does not stall on a secret
//! it cannot read. The pending-deliveries query does not look at the
//! subscription, so these gates live in `poll_deliveries`.
// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, routing::post, Router};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{test_support, webhook_worker, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;

struct Harness {
    state: AppState,
    _dir: tempfile::TempDir,
}

async fn harness() -> Harness {
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
    Harness { state, _dir: dir }
}

/// A receiver that answers 200 to every POST and counts them.
async fn receiver() -> (SocketAddr, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/hook",
            post(move |State(hits): State<Arc<AtomicUsize>>| async move {
                hits.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }),
        )
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, hits)
}

/// A subscription, registered by `creator`, whose ciphertext no key can
/// open. `cached` puts a usable secret in the runtime cache, as hydration or
/// an earlier send would.
async fn subscribed(
    state: &AppState,
    ws: WorkspaceId,
    creator: ApiTokenId,
    addr: SocketAddr,
    cached: bool,
) -> WebhookSubscriptionId {
    let sub = state
        .store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws,
            url: format!("http://{addr}/hook"),
            label: None,
            event_kinds: vec!["message_posted".into()],
            secret_ciphertext: "not-a-ciphertext".into(),
            created_by_token: Some(creator),
        })
        .await
        .unwrap();
    if cached {
        maidan_server::webhooks::remember_webhook_secret(
            &state.webhooks.secrets,
            sub.id,
            "s".into(),
        );
    }
    sub.id
}

/// The log id of a new event in `ws`, one its admin can read, for a delivery
/// to carry: a delivery goes out only while its hook's creator can read the
/// event.
async fn logged_event(store: &dyn Store, ws: WorkspaceId, name: &str) -> i64 {
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

fn cached(state: &AppState, id: WebhookSubscriptionId) -> bool {
    state.webhooks.secrets.read().unwrap().contains_key(&id)
}

#[tokio::test]
async fn a_revoked_webhook_gets_no_retries_and_an_unreadable_secret_does_not_stall_the_queue() {
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    #[allow(unsafe_code)] // edition 2024 makes env writes unsafe; test setup only
    unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
    let h = harness().await;
    let state = &h.state;
    let store = state.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace {
            name: "hooks".into(),
        })
        .await
        .unwrap()
        .id;

    let (revoked_addr, revoked_hits) = receiver().await;
    let (live_addr, live_hits) = receiver().await;
    let (locked_addr, locked_hits) = receiver().await;
    let creator = test_support::registrar(store, ws).await;
    let revoked = subscribed(state, ws, creator, revoked_addr, true).await;
    let live = subscribed(state, ws, creator, live_addr, true).await;
    let locked = subscribed(state, ws, creator, locked_addr, false).await;

    let revoked_delivery = store
        .enqueue_webhook_delivery(revoked, logged_event(store, ws, "revoked").await, "{}")
        .await
        .unwrap()
        .unwrap();
    let locked_delivery = store
        .enqueue_webhook_delivery(locked, logged_event(store, ws, "locked").await, "{}")
        .await
        .unwrap()
        .unwrap();
    let live_delivery = store
        .enqueue_webhook_delivery(live, logged_event(store, ws, "live").await, "{}")
        .await
        .unwrap()
        .unwrap();
    store
        .revoke_webhook_subscription(ws, revoked)
        .await
        .unwrap();

    webhook_worker::poll_deliveries(state, 1).await.unwrap();

    assert_eq!(
        revoked_hits.load(Ordering::SeqCst),
        0,
        "a revoked hook is never sent its queued delivery"
    );
    let row = store
        .get_webhook_delivery(revoked_delivery, ws)
        .await
        .unwrap();
    assert!(row.quarantined_at.is_some(), "and the delivery is parked");
    assert_eq!(row.attempts, 0, "without counting as an attempt");
    assert!(
        !cached(state, revoked),
        "the revoked hook's secret leaves the cache"
    );

    assert_eq!(locked_hits.load(Ordering::SeqCst), 0);
    let row = store
        .get_webhook_delivery(locked_delivery, ws)
        .await
        .unwrap();
    assert_eq!(
        row.attempts, 1,
        "an unreadable secret is a failed attempt, not a skip"
    );
    assert!(
        row.quarantined_at.is_some(),
        "and reaching the ceiling quarantines it"
    );

    assert_eq!(
        live_hits.load(Ordering::SeqCst),
        1,
        "the healthy hook behind them is delivered"
    );
    let row = store.get_webhook_delivery(live_delivery, ws).await.unwrap();
    assert!(row.delivered_at.is_some());

    assert!(
        store
            .list_pending_webhook_deliveries(100)
            .await
            .unwrap()
            .is_empty(),
        "nothing is left due to come back on the next poll"
    );
}
