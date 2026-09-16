//! Workspace handle aliases (Cluster 395): set / get / rename / lookup.
//! Both backends. A rename must not change the workspace id.

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{NewWorkspace, RoomCard};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "Room One".into(),
        })
        .await
        .expect("ws");
    let other = store
        .create_workspace(NewWorkspace {
            name: "Room Two".into(),
        })
        .await
        .expect("other");

    assert!(store
        .get_workspace_handle(ws.id)
        .await
        .expect("get none")
        .is_none());

    let set = store
        .set_workspace_handle(ws.id, "acme")
        .await
        .expect("set");
    assert_eq!(set.handle, "acme");
    assert_eq!(set.workspace_id, ws.id);

    let got = store
        .get_workspace_handle(ws.id)
        .await
        .expect("get")
        .expect("some");
    assert_eq!(got.handle, "acme");

    // Cluster 398.7: a handle is a display label, not an address — there is no
    // reverse lookup, so the rename property is asserted from the workspace side.
    let renamed = store
        .set_workspace_handle(ws.id, "renamed")
        .await
        .expect("rename");
    assert_eq!(renamed.workspace_id, ws.id);
    assert_eq!(renamed.handle, "renamed");
    let after_rename = store
        .get_workspace_handle(ws.id)
        .await
        .expect("get after rename")
        .expect("some");
    assert_eq!(
        after_rename.handle, "renamed",
        "the old handle is gone, not kept alongside the new one"
    );

    let after = store.get_workspace(ws.id).await.expect("ws still");
    assert_eq!(after.id, ws.id);
    assert_eq!(after.name, "Room One");

    let card = RoomCard::new(ws.id, Some("renamed".into()));
    assert_eq!(card.uri, format!("maidan://{}", ws.id.0));
    assert!(!card.uri.contains("renamed"));

    let err = store
        .set_workspace_handle(other.id, "renamed")
        .await
        .expect_err("taken");
    assert!(matches!(err, StoreError::Conflict(_)));

    let bad = store
        .set_workspace_handle(ws.id, "Acme")
        .await
        .expect_err("syntax");
    assert!(matches!(bad, StoreError::InvalidInput(_)));

    let missing = store
        .set_workspace_handle(maidan_types::WorkspaceId::new(), "ghost")
        .await
        .expect_err("fk");
    assert!(matches!(missing, StoreError::NotFound));
}

#[tokio::test]
async fn workspace_handles_set_get_rename_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn workspace_handles_set_get_rename_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
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
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
