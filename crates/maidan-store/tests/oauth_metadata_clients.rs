//! Metadata-document OAuth clients (migration 0172): the marked row a
//! client identified by a client metadata document gets on first use,
//! and the upsert that inserts or refreshes it. One suite, both backends.

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const METADATA_URL: &str = "https://client.example/oauth/metadata.json";

fn document(name: &str, scopes: Vec<&str>) -> NewOAuthClient {
    NewOAuthClient {
        client_id: METADATA_URL.into(),
        name: name.into(),
        redirect_uris: vec!["https://client.example/callback".into()],
        client_secret_hash: None,
        allowed_scopes: scopes.iter().map(|s| s.to_string()).collect(),
    }
}

async fn assert_upsert_marks_and_refreshes(store: &dyn Store) {
    let inserted = store
        .upsert_oauth_metadata_client(
            document("Document App", vec!["workspace:read"]),
            METADATA_URL,
        )
        .await
        .unwrap()
        .expect("the first upsert inserts the row");
    assert_eq!(inserted.client_id, METADATA_URL);
    assert_eq!(inserted.metadata_url.as_deref(), Some(METADATA_URL));
    assert_eq!(inserted.name, "Document App");

    // The getter serves the marked row, marker visible.
    let fetched = store
        .get_oauth_client_by_client_id(METADATA_URL)
        .await
        .unwrap()
        .expect("the row reads back");
    assert_eq!(fetched.metadata_url.as_deref(), Some(METADATA_URL));

    // A refresh replaces the document's fields and keeps the row's id.
    let refreshed = store
        .upsert_oauth_metadata_client(
            document("Renamed App", vec!["workspace:read", "workspace:write"]),
            METADATA_URL,
        )
        .await
        .unwrap()
        .expect("a live marked row refreshes");
    assert_eq!(refreshed.id, inserted.id);
    assert_eq!(refreshed.name, "Renamed App");
    assert_eq!(
        refreshed.allowed_scopes,
        vec!["workspace:read".to_string(), "workspace:write".to_string()]
    );
}

async fn assert_upsert_never_overwrites_a_preregistered_client(store: &dyn Store) {
    store
        .create_oauth_client(NewOAuthClient {
            client_id: "operator-client".into(),
            name: "Operator Client".into(),
            redirect_uris: vec!["https://client.example/callback".into()],
            client_secret_hash: None,
            allowed_scopes: vec!["workspace:read".into()],
        })
        .await
        .unwrap();
    let refused = store
        .upsert_oauth_metadata_client(
            NewOAuthClient {
                client_id: "operator-client".into(),
                name: "Document Takeover".into(),
                redirect_uris: vec!["https://client.example/callback".into()],
                client_secret_hash: None,
                allowed_scopes: vec!["workspace:read".into()],
            },
            "operator-client",
        )
        .await
        .unwrap();
    assert!(
        refused.is_none(),
        "a document must not overwrite a pre-registered client"
    );
    let intact = store
        .get_oauth_client_by_client_id("operator-client")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(intact.name, "Operator Client");
    assert!(intact.metadata_url.is_none());
}

async fn run_suite(store: &dyn Store) {
    assert_upsert_marks_and_refreshes(store).await;
    assert_upsert_never_overwrites_a_preregistered_client(store).await;
}

/// The assertion half of the revoked-row test, shared once the row is
/// revoked behind the store's back (revocation is an operator action;
/// there is no store call for it).
async fn assert_revoked_row_stays_revoked(store: &dyn Store) {
    let refused = store
        .upsert_oauth_metadata_client(
            document("Revoked App", vec!["workspace:read"]),
            METADATA_URL,
        )
        .await
        .unwrap();
    assert!(refused.is_none(), "a revoked client must not resurrect");
    assert!(
        store
            .get_oauth_client_by_client_id(METADATA_URL)
            .await
            .unwrap()
            .is_none(),
        "the revoked row stays out of the live read"
    );
}

#[tokio::test]
async fn oauth_metadata_client_upsert_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let operator = pool.clone();
    let store = SqliteStore::for_tests(pool);
    run_suite(&store).await;

    store
        .upsert_oauth_metadata_client(
            document("Revoked App", vec!["workspace:read"]),
            METADATA_URL,
        )
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE oauth_clients SET revoked_at = ? WHERE client_id = ?")
        .bind(chrono::Utc::now())
        .bind(METADATA_URL)
        .execute(&operator)
        .await
        .unwrap();
    assert_revoked_row_stays_revoked(&store).await;
}

#[tokio::test]
async fn oauth_metadata_client_upsert_postgres() {
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
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let operator = pool.clone();
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;

    store
        .upsert_oauth_metadata_client(
            document("Revoked App", vec!["workspace:read"]),
            METADATA_URL,
        )
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE oauth_clients SET revoked_at = NOW() WHERE client_id = $1")
        .bind(METADATA_URL)
        .execute(&operator)
        .await
        .unwrap();
    assert_revoked_row_stays_revoked(&store).await;
}
