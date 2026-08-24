//! Cluster 263: the `Maidan-Consistency-Token` response header. On a successful
//! mutation, when a read replica is configured, the server stamps the primary's
//! WAL LSN so a client can echo it on a later read. Postgres-backed (SQLite has no
//! LSN); skips when Docker is unavailable.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{consistency::CONSISTENCY_TOKEN_HEADER, router, AppState};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::{Lsn, MemberKind, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Case {
    addr: SocketAddr,
    member_id: uuid::Uuid,
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
            eprintln!("skipping consistency_token_e2e: docker unavailable ({err})");
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
    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));

    // Seed a member to target the (bypass-auth) mutation at.
    let ws = store
        .create_workspace(NewWorkspace { name: "c".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "m".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();

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
        member_id: member.id.0,
        _server: server,
        _dir: dir,
        _container: container,
    })
}

#[tokio::test]
async fn mutation_stamps_consistency_token_when_replica_enabled() {
    let Some(c) = spawn(true).await else { return };
    let base = format!("http://{}", c.addr);
    let client = reqwest::Client::new();
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
    let client = reqwest::Client::new();
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
