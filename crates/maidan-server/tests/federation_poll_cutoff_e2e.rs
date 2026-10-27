//! Federation peer polling under the hard deliveries cutoff, on both
//! backends, with two open sign-up tenants. A workspace past its deliveries
//! allowance polls none of its peers, and nothing is counted for it. The
//! other tenant's peer is polled, and each poll counts one delivery to that
//! tenant only. Postgres skips when Docker is unavailable.
// Mock peers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{http::StatusCode, Router};
use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_auth::hash_secret;
use maidan_bus::InMemoryBus;
use maidan_server::{federation_worker, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
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
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Backend {
        store: Arc::new(SqliteStore::for_tests(pool.clone())),
        search: Arc::new(maidan_search::SqliteSearch::new(pool)),
        _dir: tempfile::tempdir().unwrap(),
    }
}

fn app_state(backend: &Backend) -> AppState {
    let mut state = AppState::new(
        backend.store.clone(),
        Arc::new(LocalFsStore::new(backend._dir.path())),
        Arc::new(InMemoryBus::with_capacity(256)),
        backend.search.clone(),
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, Some(Arc::new([0x5e; 32]))),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.usage_allowance = Arc::new(UsageAllowance::parse("deliveries=10").unwrap());
    state
}

async fn signed_up(store: &dyn Store, who: &str) -> WorkspaceId {
    store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: Utc::now() + Duration::from_secs(3600),
                open: true,
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
        .unwrap()
        .workspace
        .id
}

async fn cut_off(state: &AppState, ws: WorkspaceId) {
    let store = state.store.as_ref();
    store
        .mark_over_allowance(
            &OverAllowance {
                workspace_id: ws,
                meter: UsageMeter::Deliveries,
                month: UsageMonth::of(Utc::now().date_naive()).label(),
                quantity: 11,
                allowance: 10,
                marked_at: Utc::now(),
            },
            &[],
        )
        .await
        .unwrap();
    maidan_server::usage_meters::refresh_cut_off(state).await;
    assert!(state.usage_meters.is_cut_off(ws, UsageMeter::Deliveries));
}

/// A peer that answers every request with an empty event list and counts them.
async fn peer_server() -> (SocketAddr, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let app = Router::new().fallback(move || {
        let h = h.clone();
        async move {
            h.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, axum::Json(json!([])))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, hits)
}

fn hits(n: &AtomicUsize) -> usize {
    n.load(Ordering::SeqCst)
}

fn counted(ws: WorkspaceId, taken: &[UsageMeterDay]) -> i64 {
    taken
        .iter()
        .filter(|d| d.workspace_id == ws && d.meter == UsageMeter::Deliveries)
        .map(|d| d.quantity)
        .sum()
}

async fn suite(backend: &Backend) {
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
    let state = app_state(backend);
    let store = backend.store.as_ref();
    let a = signed_up(store, "ada").await;
    let b = signed_up(store, "bea").await;
    let (a_addr, a_hits) = peer_server().await;
    let (b_addr, b_hits) = peer_server().await;
    for (ws, addr) in [(a, a_addr), (b, b_addr)] {
        let peer = store
            .create_peer(NewPeer {
                workspace_id: ws,
                remote_workspace_id: WorkspaceId(uuid::Uuid::now_v7()),
                name: format!("peer-{}", ws.0),
                base_url: format!("http://{addr}"),
                token_hash: hash_secret(&format!("inbound-{}", ws.0)),
                outbound_secret_ciphertext: None,
            })
            .await
            .unwrap();
        maidan_server::federation::remember_peer_secret(
            &state.federation.outbound_secrets,
            peer.id,
            "outbound".into(),
        );
    }
    cut_off(&state, a).await;
    let _ = state.usage_meters.take();

    for round in 1..=2 {
        federation_worker::poll_once(&state).await.unwrap();
        assert_eq!(hits(&a_hits), 0, "a cut-off workspace polls no peer");
        assert_eq!(hits(&b_hits), round, "the other tenant's peer is polled");
        let taken = state.usage_meters.take();
        assert_eq!(counted(a, &taken), 0, "a skipped poll is not counted");
        assert_eq!(counted(b, &taken), 1, "each poll counts one delivery");
    }
}

#[tokio::test]
async fn past_its_deliveries_allowance_a_workspace_does_not_poll_its_federation_peer_while_another_does_on_sqlite(
) {
    suite(&sqlite().await).await;
}

#[tokio::test]
async fn past_its_deliveries_allowance_a_workspace_does_not_poll_its_federation_peer_while_another_does_on_postgres(
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
