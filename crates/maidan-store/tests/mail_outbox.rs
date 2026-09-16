//! Durable mail outbox (Cluster 304): enqueue / atomic-lease-claim / mark
//! delivered / reschedule-or-dead-letter / DLQ count. Both backends.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
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
        workspace_id: None,
        to_address: to.into(),
        subject: "s".into(),
        body: "b".into(),
    }
}

fn mail_for(ws: maidan_types::WorkspaceId, to: &str) -> NewMailOutbox {
    NewMailOutbox {
        workspace_id: Some(ws),
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

    // DLQ ops (Cluster 306): the dead entry (`id`, "gave up") is listed, then
    // requeued -> pending + due, no longer dead + claimable again.
    let dead = store.list_dead_mail(None, 10).await.expect("list dead");
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].id, id);
    assert_eq!(dead[0].last_error.as_deref(), Some("gave up"));
    assert!(store.requeue_dead_mail(None, id).await.expect("requeue"));
    assert_eq!(store.count_dead_mail().await.expect("count3"), 0);
    let reclaimed = store
        .claim_next_due_mail(Utc::now(), 300)
        .await
        .expect("claim7")
        .expect("requeued is claimable");
    assert_eq!(reclaimed.id, id);
    assert_eq!(reclaimed.attempts, 1, "requeue reset attempts (claim -> 1)");
    assert!(
        !store.requeue_dead_mail(None, id).await.expect("requeue2"),
        "requeue only affects a dead entry"
    );
}

#[tokio::test]
async fn mail_outbox_enqueue_claim_retry_deadletter_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
    run_dlq_scope_suite(&store).await;
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
    run_dlq_scope_suite(&store).await;
}

/// Cluster 398.3: the mail DLQ is per-workspace.
///
/// `token:admin` is minted per workspace, but this query used to be global —
/// and the rows carry `to_address`, `subject` and the message body, so any
/// workspace admin could read every other tenant's outbound email, and requeue
/// it to their recipient.
async fn run_dlq_scope_suite(store: &dyn Store) {
    let ws = |name: &'static str| async move {
        store
            .create_workspace(maidan_types::NewWorkspace { name: name.into() })
            .await
            .expect("ws")
            .id
    };
    let a = ws("mail-alpha").await;
    let b = ws("mail-bravo").await;

    let mut dead = Vec::new();
    for (w, addr) in [(a, "alpha@example.test"), (b, "bravo@example.test")] {
        let id = store
            .enqueue_mail(mail_for(w, addr))
            .await
            .expect("enqueue");
        // Dead-letter it: fail past the ceiling.
        for _ in 0..10 {
            let _ = store.mark_mail_failed(id, "nope", None).await;
        }
        dead.push((w, id, addr));
    }

    // Each tenant sees only its own — and does NOT see the other's address.
    for (w, id, addr) in &dead {
        let seen = store.list_dead_mail(Some(*w), 50).await.expect("list");
        assert_eq!(seen.len(), 1, "a tenant sees only its own dead mail");
        assert_eq!(seen[0].id, *id);
        assert_eq!(&seen[0].to_address, addr);
        assert_eq!(seen[0].workspace_id, Some(*w));
    }

    // A cannot requeue B's mail even holding its exact id — the point, since a
    // requeue re-sends to B's recipient.
    let (_, b_id, _) = dead[1];
    assert!(
        !store
            .requeue_dead_mail(Some(a), b_id)
            .await
            .expect("cross-tenant requeue"),
        "workspace A must not requeue workspace B's dead mail"
    );
    assert_eq!(
        store.list_dead_mail(Some(b), 50).await.expect("b").len(),
        1,
        "B's row is untouched by A's attempt"
    );
    // B's own admin can.
    assert!(store
        .requeue_dead_mail(Some(b), b_id)
        .await
        .expect("own requeue"));

    // The operator:global view (scope None) spans both tenants.
    let (_, a_id, _) = dead[0];
    let all = store.list_dead_mail(None, 50).await.expect("global");
    assert!(
        all.iter().any(|m| m.id == a_id),
        "the global view must still see every tenant's rows"
    );
}
