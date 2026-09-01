//! Member delivery emails (Cluster 248, Arc I): set (upsert) / get / delete a
//! member's email address. Both backends. No delivery wiring yet.

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
        .create_workspace(NewWorkspace {
            name: "emails".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "m".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");

    // Unset to start.
    assert!(store
        .get_member_email(member.id)
        .await
        .expect("get none")
        .is_none());

    // Set, then read back.
    let set = store
        .set_member_email(member.id, "me@example.com")
        .await
        .expect("set");
    assert_eq!(set.member_id, member.id);
    assert_eq!(set.email, "me@example.com");
    let got = store
        .get_member_email(member.id)
        .await
        .expect("get")
        .expect("some");
    assert_eq!(got.email, "me@example.com");

    // A re-set overwrites.
    store
        .set_member_email(member.id, "new@example.com")
        .await
        .expect("re-set");
    assert_eq!(
        store
            .get_member_email(member.id)
            .await
            .expect("get2")
            .expect("some2")
            .email,
        "new@example.com"
    );

    // Delete.
    assert!(store.delete_member_email(member.id).await.expect("delete"));
    assert!(
        !store
            .delete_member_email(member.id)
            .await
            .expect("delete again"),
        "second delete removes nothing"
    );
    assert!(store
        .get_member_email(member.id)
        .await
        .expect("get after delete")
        .is_none());
}

#[tokio::test]
async fn member_email_set_get_delete_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn member_email_set_get_delete_postgres() {
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
