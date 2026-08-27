//! Durable mail outbox (Cluster 304): enqueue / atomic-lease-claim / mark
//! delivered / reschedule-or-dead-letter / DLQ count. Both backends.

use chrono::{Duration, Utc};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::NewMailOutbox;
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

fn mail(to: &str) -> NewMailOutbox {
    NewMailOutbox {
        to_address: to.into(),
        subject: "s".into(),
        body: "b".into(),
    }
}

async fn run_suite(store: &dyn Store) {
    // Enqueue, then claim: leased forward + attempts -> 1.
    let id = store
        .enqueue_mail(mail("a@example.com"))
        .await
        .expect("enqueue");
    let claimed = store
        .claim_next_due_mail(Utc::now(), 300)
        .await
        .expect("claim")
        .expect("some");
    assert_eq!(claimed.id, id);
    assert_eq!(claimed.to_address, "a@example.com");
    assert_eq!(claimed.attempts, 1);

    // Re-claiming immediately finds nothing — the row is leased 300s into the future.
    assert!(store
        .claim_next_due_mail(Utc::now(), 300)
        .await
        .expect("claim2")
        .is_none());

    // A failure with a past retry_at reschedules it -> claimable again, attempts -> 2.
    store
        .mark_mail_failed(id, "smtp down", Some(Utc::now() - Duration::seconds(1)))
        .await
        .expect("fail-retry");
    let again = store
        .claim_next_due_mail(Utc::now(), 300)
        .await
        .expect("claim3")
        .expect("some3");
    assert_eq!(again.id, id);
    assert_eq!(again.attempts, 2);

    // Dead-letter it (retry_at None) — no longer claimable, DLQ depth 1.
    assert_eq!(store.count_dead_mail().await.expect("count0"), 0);
    store
        .mark_mail_failed(id, "gave up", None)
        .await
        .expect("dead");
    assert!(store
        .claim_next_due_mail(Utc::now(), 300)
        .await
        .expect("claim4")
        .is_none());
    assert_eq!(store.count_dead_mail().await.expect("count1"), 1);

    // A second message delivered cleanly is not re-claimed and doesn't grow the DLQ.
    let id2 = store
        .enqueue_mail(mail("c@example.com"))
        .await
        .expect("enqueue2");
    let c2 = store
        .claim_next_due_mail(Utc::now(), 300)
        .await
        .expect("claim5")
        .expect("some5");
    assert_eq!(c2.id, id2);
    store.mark_mail_delivered(id2).await.expect("delivered");
    assert!(store
        .claim_next_due_mail(Utc::now(), 300)
        .await
        .expect("claim6")
        .is_none());
    assert_eq!(store.count_dead_mail().await.expect("count2"), 1);
}

#[tokio::test]
async fn mail_outbox_enqueue_claim_retry_deadletter_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn mail_outbox_enqueue_claim_retry_deadletter_postgres() {
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
