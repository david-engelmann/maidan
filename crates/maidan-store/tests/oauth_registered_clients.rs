//! Self-registered OAuth clients (RFC 7591, migration 0220/0219): the marked
//! row `POST /oauth/register` inserts, the use that keeps it, and the sweep
//! that deletes one never authorized within a day or unused for 90 days,
//! with its grants. One suite, both backends.

use std::time::Duration;

use chrono::{DateTime, Utc};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

fn client(client_id: &str) -> NewOAuthClient {
    NewOAuthClient {
        client_id: client_id.into(),
        name: "Cursor".into(),
        redirect_uris: vec!["http://localhost:8787/callback".into()],
        client_secret_hash: None,
        allowed_scopes: vec!["workspace:read".into()],
    }
}

async fn alive(store: &dyn Store, client_id: &str) -> bool {
    store
        .get_oauth_client_by_client_id(client_id)
        .await
        .unwrap()
        .is_some()
}

async fn run_suite(store: &dyn Store) {
    let t0: DateTime<Utc> = Utc::now();
    let day = chrono::Duration::hours(24);

    let registered = store
        .register_oauth_client(client("dyn-never-used"), t0)
        .await
        .unwrap();
    assert!(
        registered.registered_at.is_some(),
        "a self-registered row is marked"
    );
    assert!(registered.client_secret_hash.is_none());
    assert!(registered.metadata_url.is_none());
    let operator = store
        .create_oauth_client(client("operator-client"))
        .await
        .unwrap();
    assert!(
        operator.registered_at.is_none(),
        "an operator's row is not marked"
    );

    store
        .register_oauth_client(client("dyn-authorized"), t0)
        .await
        .unwrap();
    assert_eq!(
        store
            .count_unauthorized_registered_oauth_clients()
            .await
            .unwrap(),
        2
    );
    store
        .mark_registered_oauth_client_used("dyn-authorized", t0 + chrono::Duration::minutes(5))
        .await
        .unwrap();
    // Marking an operator's client is a no-op, never a mark.
    store
        .mark_registered_oauth_client_used("operator-client", t0)
        .await
        .unwrap();
    assert_eq!(
        store
            .count_unauthorized_registered_oauth_clients()
            .await
            .unwrap(),
        1,
        "the client that completed an exchange no longer waits"
    );

    // A grant under the authorized client, to see the sweep take it along.
    let ws = store
        .create_workspace(NewWorkspace {
            name: "sweep-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "sweep-owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let grant = store
        .create_oauth_grant(NewOAuthGrant {
            client_id: "dyn-authorized".into(),
            member_id: member.id,
            workspace_id: ws.id,
            scope: vec!["workspace:read".into()],
            lineage_id: uuid::Uuid::now_v7(),
        })
        .await
        .unwrap();

    // Within the day: nothing goes.
    let now = t0 + chrono::Duration::hours(23);
    assert_eq!(
        store
            .sweep_registered_oauth_clients(now - day, now - chrono::Duration::days(90))
            .await
            .unwrap(),
        0
    );

    // A day on: the never-authorized client goes; the authorized one and the
    // operator's stay.
    let now = t0 + chrono::Duration::hours(25);
    assert_eq!(
        store
            .sweep_registered_oauth_clients(now - day, now - chrono::Duration::days(90))
            .await
            .unwrap(),
        1
    );
    assert!(!alive(store, "dyn-never-used").await);
    assert!(alive(store, "dyn-authorized").await);
    assert!(alive(store, "operator-client").await);

    // 91 days unused: the authorized client goes too, with its grant; the
    // operator's client is never swept.
    let now = t0 + chrono::Duration::days(91);
    assert_eq!(
        store
            .sweep_registered_oauth_clients(now - day, now - chrono::Duration::days(90))
            .await
            .unwrap(),
        1
    );
    assert!(!alive(store, "dyn-authorized").await);
    assert!(
        store.get_oauth_grant(grant.id).await.unwrap().is_none(),
        "the swept client's grant went with it"
    );
    assert!(alive(store, "operator-client").await);
}

#[tokio::test]
async fn self_registered_oauth_clients_are_swept_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    run_suite(&store).await;
}

#[tokio::test]
async fn self_registered_oauth_clients_are_swept_postgres() {
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
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
