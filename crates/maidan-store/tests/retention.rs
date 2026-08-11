//! Data-retention pruning (Cluster 186): age cutoff + the at-least-once
//! delivery-cursor floor for the event log; audit + deliveries by age.

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{Event, MemberKind, NewAuditEvent, NewMember, NewWorkspace};
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

async fn workspace_with_member(store: &dyn Store, name: &str) -> maidan_types::Member {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("ws");
    store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member")
}

async fn append_event_at(store: &dyn Store, member: &maidan_types::Member, days_ago: i64) -> i64 {
    store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now() - chrono::Duration::days(days_ago),
            workspace_id: member.workspace_id,
            member: member.clone(),
        })
        .await
        .expect("append")
        .id
}

async fn run_retention_suite(store: &dyn Store) {
    let cutoff_30d = chrono::Utc::now() - chrono::Duration::days(30);

    // --- events: age cutoff prunes the old row, keeps the recent one ---
    let m1 = workspace_with_member(store, "ret-ws1").await;
    let old_id = append_event_at(store, &m1, 100).await;
    let recent_id = append_event_at(store, &m1, 0).await;
    let pruned = store
        .prune_events(cutoff_30d, i64::MAX, 5_000)
        .await
        .expect("prune events");
    assert_eq!(
        pruned, 1,
        "only the 100-day-old event is past a 30-day cutoff"
    );
    assert!(store.get_stored_event(old_id).await.is_err(), "old gone");
    assert!(
        store.get_stored_event(recent_id).await.is_ok(),
        "recent kept"
    );

    // --- events: the delivery-cursor floor keeps old events above the watermark ---
    // Two genuinely-old events; a durable consumer's cursor sits at the first.
    let m2 = workspace_with_member(store, "ret-ws2").await;
    let old_a = append_event_at(store, &m2, 100).await;
    let old_b = append_event_at(store, &m2, 100).await;
    store
        .advance_delivery_cursor("consumer-x", m2.workspace_id, old_a)
        .await
        .expect("advance");
    let floor = store.min_delivery_cursor().await.expect("min").unwrap();
    assert_eq!(floor, old_a, "floor is the lowest cursor watermark");
    // Age matches both, but the floor caps id at old_a: old_a goes, old_b stays.
    let pruned2 = store
        .prune_events(cutoff_30d, floor, 5_000)
        .await
        .expect("prune floored");
    assert_eq!(pruned2, 1, "only the event at/under the cursor is pruned");
    assert!(store.get_stored_event(old_a).await.is_err());
    assert!(
        store.get_stored_event(old_b).await.is_ok(),
        "old event above the delivery watermark is retained"
    );

    // --- audit: cutoff logic (rows land at now()) ---
    let future = chrono::Utc::now() + chrono::Duration::days(1);
    store
        .append_audit(NewAuditEvent {
            actor_id: None,
            action: "test.action".into(),
            target_kind: None,
            target_id: None,
            metadata: serde_json::json!({}),
        })
        .await
        .expect("audit");
    let past = chrono::Utc::now() - chrono::Duration::days(1);
    assert_eq!(
        store.prune_audit(past, 5_000).await.expect("audit past"),
        0,
        "a past cutoff prunes nothing"
    );
    assert_eq!(
        store
            .prune_audit(future, 5_000)
            .await
            .expect("audit future"),
        1,
        "a future cutoff prunes the row"
    );

    // --- deliveries: the query is valid and returns 0 on empty tables ---
    assert_eq!(
        store
            .prune_deliveries(future, 5_000)
            .await
            .expect("deliveries"),
        0
    );

    // No durable consumer → floor is None (prune purely by age).
    let fresh_store_cursor = store.min_delivery_cursor().await.expect("min2");
    assert!(fresh_store_cursor.is_some(), "cursor set earlier persists");
}

#[tokio::test]
async fn retention_prunes_by_age_and_respects_the_delivery_floor_sqlite() {
    let store = sqlite().await;
    // No cursors yet → None.
    assert_eq!(store.min_delivery_cursor().await.expect("min0"), None);
    run_retention_suite(&store).await;
}

#[tokio::test]
async fn retention_prunes_by_age_and_respects_the_delivery_floor_postgres() {
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
    run_retention_suite(&store).await;
}
