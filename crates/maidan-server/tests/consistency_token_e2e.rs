//! The `Maidan-Consistency-Token` response header. On a successful mutation,
//! when a read replica is configured, the server stamps the primary's WAL LSN
//! so a client can echo it on a later read. Postgres-backed (SQLite has no
//! LSN); skips when Docker is unavailable.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{consistency::CONSISTENCY_TOKEN_HEADER, router, test_support, AppState};
use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::{Lsn, MemberKind};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Case {
    addr: SocketAddr,
    member_id: uuid::Uuid,
    client: reqwest::Client,
    _server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
    _container: testcontainers::ContainerAsync<Postgres>,
}

async fn spawn(replica_enabled: bool) -> Option<Case> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));

    // The member edits its own email, so the mutation needs no admin.
    let tenant = test_support::tenant(store.as_ref(), "c").await;
    let member = tenant.member(store.as_ref(), "m", MemberKind::Human).await;

    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::for_tests(store, artifacts, bus, search);
    state.read_replica_enabled = replica_enabled;

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Some(Case {
        addr,
        member_id: member.member_id.0,
        client: member.client(),
        _server: server,
        _dir: dir,
        _container: container,
    })
}

#[tokio::test]
async fn mutation_stamps_consistency_token_when_replica_enabled() {
    let Some(c) = spawn(true).await else { return };
    let base = format!("http://{}", c.addr);
    let client = c.client.clone();
    let mid = c.member_id;

    // A successful mutation carries the token, and it parses as a pg_lsn.
    let put = client
        .put(format!("{base}/members/{mid}/email"))
        .json(&json!({ "email": "m@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let token = put
        .headers()
        .get(CONSISTENCY_TOKEN_HEADER)
        .expect("mutation carries a consistency token")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        Lsn::from_pg_str(&token).is_some(),
        "token is a valid pg_lsn: {token}"
    );

    // A read (GET) is not a mutation → no token.
    let get = client
        .get(format!("{base}/members/{mid}/email"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    assert!(
        get.headers().get(CONSISTENCY_TOKEN_HEADER).is_none(),
        "reads do not carry a consistency token"
    );
}

#[tokio::test]
async fn no_token_when_replica_disabled() {
    let Some(c) = spawn(false).await else { return };
    let base = format!("http://{}", c.addr);
    let client = c.client.clone();
    let mid = c.member_id;

    let put = client
        .put(format!("{base}/members/{mid}/email"))
        .json(&json!({ "email": "m@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    assert!(
        put.headers().get(CONSISTENCY_TOKEN_HEADER).is_none(),
        "no replica configured → no token round-trip"
    );
}
