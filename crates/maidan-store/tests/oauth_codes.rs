//! OAuth authorization code store (Cluster 104.0.1): single-use + TTL, both
//! backends behave identically.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn seed_app(store: &dyn Store) -> (WorkspaceId, AppId) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "oauth-ws".into(),
        })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let app = store
        .create_app(NewApp {
            workspace_id: ws.id,
            slug: "bot".into(),
            name: "Bot".into(),
            description: None,
            created_by: owner.id,
        })
        .await
        .unwrap();
    (ws.id, app.id)
}

/// Shared assertions exercised against both backends.
async fn assert_single_use_and_ttl(store: &dyn Store) {
    let (ws, app) = seed_app(store).await;

    // A live code: consume once returns the payload, a second consume is empty.
    store
        .insert_oauth_code(NewOAuthCode {
            code_hash: "hash-live".into(),
            app_id: app,
            workspace_id: ws,
            redirect_uri: "https://app.example/cb".into(),
            code_challenge: Some("challenge".into()),
            expires_at: Utc::now() + chrono::Duration::seconds(600),
        })
        .await
        .unwrap();

    let got = store
        .consume_oauth_code("hash-live")
        .await
        .unwrap()
        .expect("live code should be returned");
    assert_eq!(got.app_id, app);
    assert_eq!(got.workspace_id, ws);
    assert_eq!(got.redirect_uri, "https://app.example/cb");
    assert_eq!(got.code_challenge.as_deref(), Some("challenge"));

    assert!(
        store
            .consume_oauth_code("hash-live")
            .await
            .unwrap()
            .is_none(),
        "code must be single-use"
    );

    // An already-expired code is never returned (TTL).
    store
        .insert_oauth_code(NewOAuthCode {
            code_hash: "hash-expired".into(),
            app_id: app,
            workspace_id: ws,
            redirect_uri: "https://app.example/cb".into(),
            code_challenge: None,
            expires_at: Utc::now() - chrono::Duration::seconds(5),
        })
        .await
        .unwrap();
    assert!(
        store
            .consume_oauth_code("hash-expired")
            .await
            .unwrap()
            .is_none(),
        "expired code must not be returned"
    );

    // Unknown codes are simply absent.
    assert!(store.consume_oauth_code("nope").await.unwrap().is_none());
}

#[tokio::test]
async fn oauth_codes_single_use_and_ttl_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);
    assert_single_use_and_ttl(&store).await;
}

#[tokio::test]
async fn oauth_codes_single_use_and_ttl_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping oauth_codes postgres: docker unavailable ({err})");
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
    let store = PostgresStore::new(pool);
    assert_single_use_and_ttl(&store).await;
}
