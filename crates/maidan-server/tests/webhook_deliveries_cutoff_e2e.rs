//! Outbound webhook deliveries under the hard deliveries cutoff, on both
//! backends, with two open sign-up tenants. A workspace past its deliveries
//! allowance sends nothing: its first attempts and retries are deferred with
//! their attempt counts as they were, and nothing is counted. The other
//! tenant's deliveries go out, and each attempt, failed or not, counts one
//! delivery. Postgres skips when Docker is unavailable.
// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{extract::State, http::StatusCode, routing::post, Router};
use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{webhook_worker, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations, AutomationDeliveryFilter};
use maidan_types::*;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct Backend {
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
    _dir: tempfile::TempDir,
}

async fn sqlite() -> Backend {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Backend {
        store: Arc::new(SqliteStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::SqliteSearch::new(pool)),
        _dir: tempfile::tempdir().unwrap(),
    }
}

fn state_on(backend: &Backend) -> AppState {
    let mut state = AppState::for_tests(
        backend.store.clone(),
        Arc::new(LocalFsStore::new(backend._dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        backend.search.clone(),
    );
    state.usage_allowance = Arc::new(UsageAllowance::parse("deliveries=10").unwrap());
    state
}

async fn signed_up(store: &dyn Store, who: &str, open: bool) -> (WorkspaceId, MemberId) {
    let outcome = store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: Utc::now() + Duration::from_secs(3600),
                open,
            },
            Box::new(|core| NewAuditEvent {
                scope: AuditScope::Workspace(core.workspace.id),
                actor_id: Some(core.member.id),
                action: "workspace.signup".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(core.workspace.id.0),
                metadata: json!({}),
            }),
            Box::new(|session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: json!({}),
            }),
        )
        .await
        .unwrap();
    (outcome.workspace.id, outcome.member.id)
}

/// A receiver that answers `status` to every POST and counts them.
async fn receiver(status: StatusCode) -> (SocketAddr, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/hook",
            post(move |State(hits): State<Arc<AtomicUsize>>| async move {
                hits.fetch_add(1, Ordering::SeqCst);
                status
            }),
        )
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, hits)
}

async fn subscribed(state: &AppState, ws: WorkspaceId, addr: SocketAddr) -> WebhookSubscriptionId {
    let sub = state
        .store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws,
            url: format!("http://{addr}/hook"),
            label: None,
            event_kinds: vec!["message_posted".into()],
            secret_ciphertext: "x".into(),
        })
        .await
        .unwrap();
    maidan_server::webhooks::remember_webhook_secret(&state.webhooks.secrets, sub.id, "s".into());
    sub.id
}

/// Deliveries counted for `ws` since the last take.
fn counted(state: &AppState, ws: WorkspaceId) -> i64 {
    state
        .usage_meters
        .take()
        .into_iter()
        .filter(|d| d.workspace_id == ws && d.meter == UsageMeter::Deliveries)
        .map(|d| d.quantity)
        .sum()
}

async fn suite(backend: &Backend) {
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
    let state = state_on(backend);
    let store = backend.store.as_ref();
    let (a, _) = signed_up(store, "ada", true).await;
    let (b, _) = signed_up(store, "bea", true).await;
    let (a_addr, a_hits) = receiver(StatusCode::OK).await;
    let (b_addr, b_hits) = receiver(StatusCode::OK).await;
    let (down_addr, down_hits) = receiver(StatusCode::SERVICE_UNAVAILABLE).await;
    let a_sub = subscribed(&state, a, a_addr).await;
    let b_sub = subscribed(&state, b, b_addr).await;
    let b_down = subscribed(&state, b, down_addr).await;

    let a_fresh = store
        .enqueue_webhook_delivery(a_sub, 1, "{}")
        .await
        .unwrap();
    let a_retry = store
        .enqueue_webhook_delivery(a_sub, 2, "{}")
        .await
        .unwrap();
    store
        .record_webhook_delivery_attempt(
            a_retry,
            "HTTP 503",
            Utc::now() - chrono::Duration::seconds(5),
        )
        .await
        .unwrap();
    store
        .enqueue_webhook_delivery(b_sub, 3, "{}")
        .await
        .unwrap();
    store
        .enqueue_webhook_delivery(b_down, 4, "{}")
        .await
        .unwrap();

    let month = UsageMonth::of(Utc::now().date_naive()).label();
    store
        .mark_over_allowance(
            &OverAllowance {
                workspace_id: a,
                meter: UsageMeter::Deliveries,
                month,
                quantity: 11,
                allowance: 10,
                marked_at: Utc::now(),
            },
            &[],
        )
        .await
        .unwrap();
    maidan_server::usage_meters::refresh_cut_off(&state).await;
    assert!(state.usage_meters.is_cut_off(a, UsageMeter::Deliveries));
    assert!(!state.usage_meters.is_cut_off(b, UsageMeter::Deliveries));
    let _ = state.usage_meters.take();

    webhook_worker::poll_deliveries(&state, 16).await.unwrap();

    assert_eq!(
        a_hits.load(Ordering::SeqCst),
        0,
        "a cut-off workspace sends nothing"
    );
    let parked = store
        .list_webhook_deliveries(a, AutomationDeliveryFilter::Pending, 100)
        .await
        .unwrap();
    assert_eq!(
        parked.len(),
        2,
        "both are kept, not dropped or dead-lettered"
    );
    for row in &parked {
        let before = if row.id == a_fresh { 0 } else { 1 };
        assert_eq!(row.attempts, before, "parking is not an attempt");
        assert!(
            row.next_attempt_at > Utc::now() + chrono::Duration::minutes(10),
            "parked past the next polls"
        );
    }
    assert!(store
        .list_pending_webhook_deliveries(100)
        .await
        .unwrap()
        .iter()
        .all(|d| d.subscription_id != a_sub));

    assert_eq!(
        b_hits.load(Ordering::SeqCst),
        1,
        "the other tenant is delivered"
    );
    assert_eq!(
        down_hits.load(Ordering::SeqCst),
        1,
        "and its failing hook is tried"
    );
    let deliveries = state.usage_meters.take();
    let of = |ws: WorkspaceId| -> i64 {
        deliveries
            .iter()
            .filter(|d| d.workspace_id == ws && d.meter == UsageMeter::Deliveries)
            .map(|d| d.quantity)
            .sum()
    };
    assert_eq!(of(a), 0, "a parked delivery is not counted");
    assert_eq!(of(b), 2, "each attempt counts, the failed one too");
    assert_eq!(counted(&state, b), 0, "counted once, not again");
}

#[tokio::test]
async fn past_its_deliveries_allowance_a_workspace_sends_no_webhooks_while_another_does_on_sqlite()
{
    suite(&sqlite().await).await;
}

#[tokio::test]
async fn past_its_deliveries_allowance_a_workspace_sends_no_webhooks_while_another_does_on_postgres(
) {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    suite(&Backend {
        store: Arc::new(PostgresStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::PostgresSearch::new(pool)),
        _dir: tempfile::tempdir().unwrap(),
    })
    .await;
}

/// The cutoff is read at send time, per workspace: a delivery enqueued before
/// its workspace was cut off, then replayed by its own operator after, still
/// sends nothing, while the other tenant's failing hook is retried and
/// counted only to that tenant.
#[tokio::test]
async fn a_delivery_enqueued_before_the_cutoff_and_replayed_after_it_still_sends_nothing() {
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
    let backend = sqlite().await;
    let state = state_on(&backend);
    let store = backend.store.as_ref();
    let (a, _) = signed_up(store, "ada", true).await;
    let (b, _) = signed_up(store, "bea", true).await;
    let (a_addr, a_hits) = receiver(StatusCode::OK).await;
    let (down_addr, down_hits) = receiver(StatusCode::INTERNAL_SERVER_ERROR).await;
    let a_sub = subscribed(&state, a, a_addr).await;
    let b_down = subscribed(&state, b, down_addr).await;

    // Enqueued while A is under its allowance, then dead-lettered.
    let a_dead = store
        .enqueue_webhook_delivery(a_sub, 1, "{}")
        .await
        .unwrap();
    store.quarantine_webhook_delivery(a_dead).await.unwrap();
    store
        .enqueue_webhook_delivery(b_down, 2, "{}")
        .await
        .unwrap();

    let month = UsageMonth::of(Utc::now().date_naive()).label();
    store
        .mark_over_allowance(
            &OverAllowance {
                workspace_id: a,
                meter: UsageMeter::Deliveries,
                month,
                quantity: 11,
                allowance: 10,
                marked_at: Utc::now(),
            },
            &[],
        )
        .await
        .unwrap();
    maidan_server::usage_meters::refresh_cut_off(&state).await;
    let _ = state.usage_meters.take();

    // A's operator replays the dead delivery after the cutoff.
    store.replay_webhook_delivery(a_dead, a).await.unwrap();
    webhook_worker::poll_deliveries(&state, 16).await.unwrap();

    assert_eq!(
        a_hits.load(Ordering::SeqCst),
        0,
        "a replay does not outrank the cutoff"
    );
    let parked = store.get_webhook_delivery(a_dead, a).await.unwrap();
    assert_eq!(parked.attempts, 0, "parking is not an attempt");
    assert!(parked.delivered_at.is_none() && parked.quarantined_at.is_none());
    assert_eq!(
        down_hits.load(Ordering::SeqCst),
        1,
        "B's failing hook is tried"
    );
    let deliveries = state.usage_meters.take();
    let of = |ws: WorkspaceId| -> i64 {
        deliveries
            .iter()
            .filter(|d| d.workspace_id == ws && d.meter == UsageMeter::Deliveries)
            .map(|d| d.quantity)
            .sum()
    };
    assert_eq!(of(a), 0, "B's failures are never charged to A");
    assert_eq!(of(b), 1, "B's failed attempt counts to B");
}
