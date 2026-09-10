//! Web Push subscription store (Cluster 366, N1): add (upsert on endpoint) / list
//! / recipient-scoped delete. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewMember, NewPushSubscription, NewWorkspace};
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
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let other = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "b".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member2");

    assert!(store
        .list_push_subscriptions(member.id)
        .await
        .expect("list")
        .is_empty());

    let sub = store
        .add_push_subscription(NewPushSubscription {
            member_id: member.id,
            endpoint: "https://push.example.com/a".into(),
            p256dh: "key1".into(),
            auth: "auth1".into(),
        })
        .await
        .expect("add");
    assert_eq!(sub.member_id, member.id);
    assert_eq!(sub.p256dh, "key1");
    assert_eq!(
        store
            .list_push_subscriptions(member.id)
            .await
            .expect("list")
            .len(),
        1
    );

    // Upsert on (member, endpoint): re-registering the same endpoint refreshes keys.
    let refreshed = store
        .add_push_subscription(NewPushSubscription {
            member_id: member.id,
            endpoint: "https://push.example.com/a".into(),
            p256dh: "key2".into(),
            auth: "auth2".into(),
        })
        .await
        .expect("re-add");
    assert_eq!(refreshed.id, sub.id, "same endpoint -> same row");
    assert_eq!(refreshed.p256dh, "key2");
    assert_eq!(
        store
            .list_push_subscriptions(member.id)
            .await
            .expect("list")
            .len(),
        1,
        "upsert, not a new row"
    );

    // A second device is a distinct row.
    store
        .add_push_subscription(NewPushSubscription {
            member_id: member.id,
            endpoint: "https://push.example.com/b".into(),
            p256dh: "key3".into(),
            auth: "auth3".into(),
        })
        .await
        .expect("add2");
    assert_eq!(
        store
            .list_push_subscriptions(member.id)
            .await
            .expect("list")
            .len(),
        2
    );

    // Delete is recipient-scoped: another member can't remove this one.
    assert!(!store
        .delete_push_subscription(other.id, sub.id)
        .await
        .expect("del wrong member"));
    assert_eq!(
        store
            .list_push_subscriptions(member.id)
            .await
            .expect("list")
            .len(),
        2
    );

    // The owner removes it; second delete is a no-op.
    assert!(store
        .delete_push_subscription(member.id, sub.id)
        .await
        .expect("del"));
    assert!(!store
        .delete_push_subscription(member.id, sub.id)
        .await
        .expect("del again"));
    assert_eq!(
        store
            .list_push_subscriptions(member.id)
            .await
            .expect("list")
            .len(),
        1
    );
}

#[tokio::test]
async fn push_subscription_add_list_delete_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn push_subscription_add_list_delete_postgres() {
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
