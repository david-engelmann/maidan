//! Named-secret store (Cluster 371, Wave 2 #19): create/rotate/list/get/delete.
//! The store holds ciphertext (encryption is the route layer's job); this checks
//! the CRUD + the upsert-rotates contract. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewMember, NewSecret, NewWorkspace};
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
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");

    let secret = store
        .create_secret(NewSecret {
            workspace_id: ws.id,
            name: "api-key".into(),
            value_ciphertext: "ct-1".into(),
            created_by: member.id,
        })
        .await
        .expect("create");
    assert_eq!(secret.name, "api-key");

    // The ciphertext reads back; metadata list carries no value.
    assert_eq!(
        store
            .get_secret_ciphertext(ws.id, "api-key")
            .await
            .expect("get"),
        Some("ct-1".to_string())
    );
    let list = store.list_secrets(ws.id).await.expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "api-key");

    // Re-creating the same name rotates the value (upsert), not a duplicate.
    let rotated = store
        .create_secret(NewSecret {
            workspace_id: ws.id,
            name: "api-key".into(),
            value_ciphertext: "ct-2".into(),
            created_by: member.id,
        })
        .await
        .expect("rotate");
    assert_eq!(rotated.id, secret.id, "same row, rotated in place");
    assert_eq!(
        store
            .get_secret_ciphertext(ws.id, "api-key")
            .await
            .expect("get"),
        Some("ct-2".to_string())
    );
    assert_eq!(store.list_secrets(ws.id).await.expect("list").len(), 1);

    // An unknown name resolves to None; delete is idempotent.
    assert!(store
        .get_secret_ciphertext(ws.id, "nope")
        .await
        .expect("get")
        .is_none());
    assert!(store.delete_secret(ws.id, "api-key").await.expect("delete"));
    assert!(!store
        .delete_secret(ws.id, "api-key")
        .await
        .expect("delete again"));
    assert!(store.list_secrets(ws.id).await.expect("list").is_empty());
}

#[tokio::test]
async fn secret_crud_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn secret_crud_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
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
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
