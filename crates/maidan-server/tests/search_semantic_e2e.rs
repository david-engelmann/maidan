//! Postgres semantic search over HTTP (`mode=semantic`).

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_search::{hash_embedding, model_name, PostgresSearch, Search};
use maidan_server::{router, AppState};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
use reqwest::StatusCode;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn http_semantic_search_ranks_by_embedding_similarity() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping semantic e2e: docker unavailable ({err})");
            return;
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
    let search: Arc<dyn Search> = Arc::new(PostgresSearch::new(pool));
    let embedding_provider: Arc<dyn maidan_search::EmbeddingProvider> =
        Arc::new(maidan_search::HashV1Provider);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "semantic-ws".into(),
        })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let th = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let target_body = "cosmic radiation monitoring station alpha";
    let other_body = "weekly lunch menu for the cafeteria";
    let target = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: target_body.into(),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    let other = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: other_body.into(),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    search
        .upsert_embedding(target.id, model_name(), &hash_embedding(target_body))
        .await
        .unwrap();
    search
        .upsert_embedding(other.id, model_name(), &hash_embedding(other_body))
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let app = router(AppState::new(
        store,
        artifacts,
        bus,
        search,
        embedding_provider,
        true,
        maidan_server::FederationRuntime::new(true, None),
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
        None,
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let hits: Vec<serde_json::Value> = client
        .get(format!("{base}/workspaces/{}/search", ws.id.0))
        .query(&[("q", target_body), ("mode", "semantic")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!hits.is_empty());
    assert_eq!(
        hits[0]["message_id"].as_str().unwrap(),
        target.id.0.to_string()
    );
    assert!((hits[0]["rank"].as_f64().unwrap() - 1.0).abs() < 1e-6);

    server.abort();
}

#[tokio::test]
async fn sqlite_rejects_semantic_mode() {
    let (addr, client, server, _dir) = spawn_sqlite().await;
    let base = format!("http://{addr}");
    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&serde_json::json!({"name": "ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let resp = client
        .get(format!(
            "{base}/workspaces/{workspace_id}/search?q=hello&mode=semantic"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    server.abort();
}

async fn spawn_sqlite() -> (
    std::net::SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    use maidan_search::SqliteSearch;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use sqlx::sqlite::SqlitePoolOptions;

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
    let search: Arc<dyn Search> = Arc::new(SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(256));
    let app = router(AppState::for_tests(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    (addr, client, server, dir)
}
