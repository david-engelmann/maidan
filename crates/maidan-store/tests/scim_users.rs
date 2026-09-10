//! SCIM provisioning-link store (Cluster 366, SCIM-as-OIDC-P3): create / get /
//! list / update (active + externalId) / delete. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewMember, NewWorkspace};
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
            handle: "jdoe".into(),
            display_name: Some("J Doe".into()),
            kind: MemberKind::Human,
        })
        .await
        .expect("member");

    assert!(store.get_scim_user(member.id).await.expect("get").is_none());
    assert!(store.list_scim_users(ws.id).await.expect("list").is_empty());

    let link = store
        .create_scim_user(member.id, ws.id, Some("ext-1"), true)
        .await
        .expect("create");
    assert_eq!(link.external_id.as_deref(), Some("ext-1"));
    assert!(link.active);
    assert_eq!(store.list_scim_users(ws.id).await.expect("list").len(), 1);

    // Deactivate + change externalId.
    let updated = store
        .update_scim_user(member.id, Some("ext-2"), false)
        .await
        .expect("update")
        .expect("row");
    assert!(!updated.active);
    assert_eq!(updated.external_id.as_deref(), Some("ext-2"));
    assert!(updated.updated_at >= link.updated_at);

    // Update of a non-SCIM member is None.
    let other = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "other".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member2");
    assert!(store
        .update_scim_user(other.id, None, true)
        .await
        .expect("update none")
        .is_none());

    // Delete: true once, false after.
    assert!(store.delete_scim_user(member.id).await.expect("del"));
    assert!(!store.delete_scim_user(member.id).await.expect("del again"));
    assert!(store.get_scim_user(member.id).await.expect("get").is_none());
}

#[tokio::test]
async fn scim_user_crud_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn scim_user_crud_postgres() {
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
