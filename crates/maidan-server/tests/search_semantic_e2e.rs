//! Semantic search over HTTP (`mode=semantic`) on Postgres and SQLite.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_search::{hash_embedding, model_name, PostgresSearch, Search};
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
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
            content: None,
        })
        .await
        .unwrap();
    let other = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: other_body.into(),
            metadata: serde_json::json!({}),
            content: None,
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
        false,
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
async fn http_semantic_search_respects_channel_and_kind_facets() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping semantic facet e2e: docker unavailable ({err})");
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
            name: "semantic-facet-ws".into(),
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
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let general = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let release = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "release".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    let general_body = "cosmic radiation monitoring station alpha";
    let release_body = "weekly lunch menu for the cafeteria";

    let general_th = store
        .create_thread(NewThread {
            channel_id: general.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let general_msg = store
        .post_message(NewMessage {
            thread_id: general_th.id,
            author_id: alice.id,
            body: general_body.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    let release_th = store
        .create_thread(NewThread {
            channel_id: release.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let release_msg = store
        .post_message(NewMessage {
            thread_id: release_th.id,
            author_id: bot.id,
            body: release_body.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    search
        .upsert_embedding(general_msg.id, model_name(), &hash_embedding(general_body))
        .await
        .unwrap();
    search
        .upsert_embedding(release_msg.id, model_name(), &hash_embedding(release_body))
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
        false,
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
    let general_ch = general.id.0.to_string();

    let hits: Vec<serde_json::Value> = client
        .get(format!("{base}/workspaces/{}/search", ws.id.0))
        .query(&[
            ("q", general_body),
            ("mode", "semantic"),
            ("channel", general_ch.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0]["message_id"].as_str().unwrap(),
        general_msg.id.0.to_string()
    );

    let hits: Vec<serde_json::Value> = client
        .get(format!("{base}/workspaces/{}/search", ws.id.0))
        .query(&[("q", release_body), ("mode", "semantic"), ("kind", "human")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!hits
        .iter()
        .any(|h| { h["message_id"].as_str() == Some(release_msg.id.0.to_string().as_str()) }));

    let hits: Vec<serde_json::Value> = client
        .get(format!("{base}/workspaces/{}/search", ws.id.0))
        .query(&[("q", release_body), ("mode", "semantic"), ("kind", "agent")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0]["message_id"].as_str().unwrap(),
        release_msg.id.0.to_string()
    );

    server.abort();
}

#[tokio::test]
async fn sqlite_http_semantic_search_ranks_by_embedding_similarity() {
    use maidan_search::{hash_embedding, model_name, sqlite_pool_options, SqliteSearch};
    use maidan_store::{run_sqlite_migrations, SqliteStore};

    let pool = sqlite_pool_options()
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
    let embedding_provider: Arc<dyn maidan_search::EmbeddingProvider> =
        Arc::new(maidan_search::HashV1Provider);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "sqlite-semantic-ws".into(),
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
    let target_body = "sqlite semantic target phrase";
    let other_body = "unrelated sqlite lunch special";
    let target = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: target_body.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    let other = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: other_body.into(),
            metadata: serde_json::json!({}),
            content: None,
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
        false,
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

    server.abort();
}

#[tokio::test]
async fn sqlite_http_semantic_search_honors_embedding_model_param() {
    use maidan_search::{hash_embedding, model_name, sqlite_pool_options, SqliteSearch};
    use maidan_store::{run_sqlite_migrations, SqliteStore};

    const LEGACY_MODEL: &str = "legacy-v1";

    let pool = sqlite_pool_options()
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
    let embedding_provider: Arc<dyn maidan_search::EmbeddingProvider> =
        Arc::new(maidan_search::HashV1Provider);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "per-model-ws".into(),
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
    let default_body = "indexed under hash-v1 only";
    let legacy_body = "indexed under legacy-v1 only";
    let default_msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: default_body.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    let legacy_msg = store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: legacy_body.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    search
        .upsert_embedding(default_msg.id, model_name(), &hash_embedding(default_body))
        .await
        .unwrap();
    search
        .upsert_embedding(legacy_msg.id, LEGACY_MODEL, &hash_embedding(legacy_body))
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
        false,
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
    let search_url = format!("{base}/workspaces/{}/search", ws.id.0);

    let default_hits: Vec<serde_json::Value> = client
        .get(&search_url)
        .query(&[("q", legacy_body), ("mode", "semantic")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !default_hits
            .iter()
            .any(|h| h["message_id"].as_str() == Some(legacy_msg.id.0.to_string().as_str())),
        "legacy-only row must not appear when querying the default model table"
    );

    let legacy_hits: Vec<serde_json::Value> = client
        .get(&search_url)
        .query(&[
            ("q", legacy_body),
            ("mode", "semantic"),
            ("embedding_model", LEGACY_MODEL),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(legacy_hits.len(), 1);
    assert_eq!(
        legacy_hits[0]["message_id"].as_str().unwrap(),
        legacy_msg.id.0.to_string()
    );
    assert_eq!(
        legacy_hits[0]["embedding_model"].as_str(),
        Some(LEGACY_MODEL)
    );

    let default_only: Vec<serde_json::Value> = client
        .get(&search_url)
        .query(&[("q", default_body), ("mode", "semantic")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(default_only.len(), 1);
    assert_eq!(
        default_only[0]["message_id"].as_str().unwrap(),
        default_msg.id.0.to_string()
    );

    server.abort();
}
