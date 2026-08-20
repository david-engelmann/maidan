//! Email digest data model (Cluster 254, Arc I): delivery-mode preference,
//! digest watermark, and the "due for digest" enumeration. Both backends.

use chrono::{Duration, Utc};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    EmailDeliveryMode, EventKind, MemberId, MemberKind, NewMember, NewNotification, NewWorkspace,
    WorkspaceId,
};
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

async fn add_member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member")
        .id
}

async fn add_unread(store: &dyn Store, ws: WorkspaceId, member: MemberId, source_log_id: i64) {
    store
        .create_notification(NewNotification {
            workspace_id: ws,
            member_id: member,
            kind: EventKind::MentionRecorded,
            source_log_id,
            channel_id: None,
            thread_id: None,
            message_id: None,
            actor_id: None,
        })
        .await
        .expect("notification");
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "digest".into(),
        })
        .await
        .expect("ws");

    // A: digest mode + email + unread -> due.
    let a = add_member(store, ws.id, "a").await;
    store.set_member_email(a, "a@example.com").await.unwrap();
    // Default before any set is Immediate.
    assert_eq!(
        store.get_delivery_mode(a).await.unwrap(),
        EmailDeliveryMode::Immediate
    );
    store
        .set_delivery_mode(a, EmailDeliveryMode::Digest)
        .await
        .unwrap();
    assert_eq!(
        store.get_delivery_mode(a).await.unwrap(),
        EmailDeliveryMode::Digest
    );
    add_unread(store, ws.id, a, 1).await;
    add_unread(store, ws.id, a, 2).await;

    // B: immediate mode (default) + email + unread -> NOT due (mode filter).
    let b = add_member(store, ws.id, "b").await;
    store.set_member_email(b, "b@example.com").await.unwrap();
    add_unread(store, ws.id, b, 3).await;

    // C: digest mode + NO email + unread -> NOT due (email join).
    let c = add_member(store, ws.id, "c").await;
    store
        .set_delivery_mode(c, EmailDeliveryMode::Digest)
        .await
        .unwrap();
    add_unread(store, ws.id, c, 4).await;

    // D: digest mode + email but all-read -> NOT due (unread filter).
    let d = add_member(store, ws.id, "d").await;
    store.set_member_email(d, "d@example.com").await.unwrap();
    store
        .set_delivery_mode(d, EmailDeliveryMode::Digest)
        .await
        .unwrap();
    add_unread(store, ws.id, d, 5).await;
    store.mark_all_notifications_read(d).await.unwrap();

    // Only A is due, with both unread notifications counted.
    let due = store.members_due_for_digest(100).await.unwrap();
    assert_eq!(due.len(), 1, "only A is due");
    assert_eq!(due[0].member_id, a);
    assert_eq!(due[0].email, "a@example.com");
    assert_eq!(due[0].unread_count, 2);

    // A watermark in the future drops A (nothing created after it).
    store
        .set_last_digest_at(a, Utc::now() + Duration::hours(1))
        .await
        .unwrap();
    assert!(
        store.members_due_for_digest(100).await.unwrap().is_empty(),
        "future watermark clears A"
    );

    // A watermark in the past brings A back (its unreads are all newer).
    store
        .set_last_digest_at(a, Utc::now() - Duration::hours(1))
        .await
        .unwrap();
    let due = store.members_due_for_digest(100).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].member_id, a);
    assert_eq!(due[0].unread_count, 2);
}

#[tokio::test]
async fn email_digest_model_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn email_digest_model_postgres() {
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
