//! Data-retention pruning: age cutoff + the at-least-once delivery-cursor floor
//! for the event log; audit + deliveries by age.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Event, EventKind, MemberKind, NewAuditEvent, NewMember, NewNotification, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;
use std::future::Future;

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
    let floor = store
        .min_delivery_cursor(cutoff_30d)
        .await
        .expect("min")
        .unwrap();
    assert_eq!(floor, old_a, "floor is the lowest cursor watermark");
    // A cursor that has not advanced since the cutoff no longer holds the
    // floor: an abandoned consumer id must not stop pruning forever.
    let later = chrono::Utc::now() + chrono::Duration::days(1);
    assert_eq!(
        store.min_delivery_cursor(later).await.expect("stale"),
        None,
        "a cursor idle since before `advanced_since` does not pin retention"
    );
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
            scope: maidan_types::AuditScope::Instance,
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
    let fresh_store_cursor = store.min_delivery_cursor(cutoff_30d).await.expect("min2");
    assert!(fresh_store_cursor.is_some(), "cursor set earlier persists");
}

async fn run_notification_retention<F, Fut>(store: &dyn Store, age: F)
where
    F: Fn(uuid::Uuid) -> Fut,
    Fut: Future<Output = ()>,
{
    let open = workspace_with_member(store, "notes-open").await;
    let held_member = workspace_with_member(store, "notes-held").await;

    async fn notify(
        store: &dyn Store,
        member: &maidan_types::Member,
        source: i64,
    ) -> maidan_types::Notification {
        store
            .create_notification(NewNotification {
                workspace_id: member.workspace_id,
                member_id: member.id,
                kind: EventKind::MemberJoined,
                source_log_id: source,
                channel_id: None,
                thread_id: None,
                message_id: None,
                actor_id: None,
            })
            .await
            .expect("notification")
    }

    let old_read = notify(store, &open, 1).await;
    let unread = notify(store, &open, 2).await;
    let snoozed = notify(store, &open, 3).await;
    let lapsed = notify(store, &open, 4).await;
    let recent = notify(store, &open, 5).await;
    let held_read = notify(store, &held_member, 1).await;

    for note in [&old_read, &snoozed, &lapsed, &recent, &held_read] {
        assert!(store
            .mark_notification_read(note.member_id, note.id)
            .await
            .expect("read"));
    }
    assert!(store
        .snooze_notification(
            snoozed.member_id,
            snoozed.id,
            chrono::Utc::now() + chrono::Duration::days(7),
        )
        .await
        .expect("snooze"));
    assert!(store
        .snooze_notification(
            lapsed.member_id,
            lapsed.id,
            chrono::Utc::now() - chrono::Duration::days(2),
        )
        .await
        .expect("lapsed snooze"));
    for note in [&old_read, &unread, &snoozed, &lapsed, &held_read] {
        age(note.id.0).await;
    }
    store
        .place_legal_hold(held_member.workspace_id, "matter", None)
        .await
        .expect("hold");

    let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
    assert_eq!(
        store
            .prune_notifications(cutoff, 5_000)
            .await
            .expect("prune"),
        1,
        "only the open workspace's old read notification, with no snooze set"
    );
    let ids: Vec<_> = store
        .list_notifications(open.id, false, 20)
        .await
        .expect("list")
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert!(
        !ids.contains(&old_read.id),
        "the old read notification is gone"
    );
    assert!(ids.contains(&unread.id), "an unread notification stays");
    // The inbox hides a snooze that is still ahead, so the row is checked
    // directly. Marking read is idempotent and reports whether the row exists.
    assert!(
        store
            .mark_notification_read(snoozed.member_id, snoozed.id)
            .await
            .expect("snoozed still there"),
        "a snoozed notification stays"
    );
    assert!(
        !ids.contains(&snoozed.id),
        "a future snooze is hidden from the inbox, not deleted"
    );
    assert!(
        ids.contains(&lapsed.id),
        "a snooze that has already lapsed still keeps the row"
    );
    assert!(
        ids.contains(&recent.id),
        "a read notification inside the window stays"
    );
    assert_eq!(
        store
            .list_notifications(held_member.id, false, 20)
            .await
            .expect("held list")
            .len(),
        1,
        "a held workspace keeps its old read notification"
    );

    let hold = store
        .list_legal_holds()
        .await
        .expect("holds")
        .into_iter()
        .find(|h| h.workspace_id == held_member.workspace_id)
        .expect("hold");
    store
        .lift_legal_hold(held_member.workspace_id, hold.id)
        .await
        .expect("lift");
    assert_eq!(
        store
            .prune_notifications(cutoff, 5_000)
            .await
            .expect("after lift"),
        1,
        "lifting the hold releases that read notification, and nothing else"
    );
    assert!(store
        .list_notifications(held_member.id, false, 20)
        .await
        .expect("held list")
        .is_empty());
    assert_eq!(
        store
            .list_notifications(open.id, false, 20)
            .await
            .expect("list")
            .len(),
        3,
        "unread, lapsed-snooze and recent rows are still in the inbox"
    );
}

#[tokio::test]
async fn retention_prunes_by_age_and_respects_the_delivery_floor_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    // No cursors yet → None.
    let long_ago = chrono::Utc::now() - chrono::Duration::days(365);
    assert_eq!(
        store.min_delivery_cursor(long_ago).await.expect("min0"),
        None
    );
    run_retention_suite(&store).await;
    let aged = chrono::Utc::now() - chrono::Duration::days(100);
    run_notification_retention(&store, |id| {
        let pool = pool.clone();
        async move {
            sqlx::query("UPDATE maidan_notifications SET created_at = ? WHERE id = ?")
                .bind(aged)
                .bind(id)
                .execute(&pool)
                .await
                .expect("age");
        }
    })
    .await;
}

#[tokio::test]
async fn sqlite_cursor_later_the_same_day_still_holds_the_retention_floor() {
    // `advance_delivery_cursor` stamps `updated_at` with SQLite
    // `CURRENT_TIMESTAMP` (`YYYY-MM-DD HH:MM:SS`). The cutoff is bound as
    // RFC3339 (`YYYY-MM-DDTHH:MM:SS+00:00`). On the cutoff's calendar day a
    // later cursor sorts first as text, because ` ` < `T`, so it looks idle
    // and the floor disappears.
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    let member = workspace_with_member(&store, "cursor-day").await;
    let delivered = append_event_at(&store, &member, 100).await;
    let pending = append_event_at(&store, &member, 100).await;
    store
        .advance_delivery_cursor("consumer-day", member.workspace_id, delivered)
        .await
        .expect("advance");
    sqlx::query(
        "UPDATE maidan_delivery_cursor SET updated_at = '2026-09-01 20:00:00'
         WHERE consumer_id = 'consumer-day'",
    )
    .execute(&pool)
    .await
    .expect("stamp");
    let cutoff = chrono::DateTime::parse_from_rfc3339("2026-09-01T18:00:00+00:00")
        .unwrap()
        .with_timezone(&chrono::Utc);

    assert_eq!(
        store.min_delivery_cursor(cutoff).await.expect("floor"),
        Some(delivered),
        "a cursor that moved after the cutoff still holds the floor"
    );
    store
        .prune_workspace_events(member.workspace_id, cutoff, 100)
        .await
        .expect("prune");
    assert!(
        store.get_stored_event(delivered).await.is_err(),
        "the event at the cursor is already delivered"
    );
    assert!(
        store.get_stored_event(pending).await.is_ok(),
        "an event past the cursor is still owed to that consumer"
    );
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
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
    let store = PostgresStore::for_tests(pool.clone());
    run_retention_suite(&store).await;
    let aged = chrono::Utc::now() - chrono::Duration::days(100);
    run_notification_retention(&store, |id| {
        let pool = pool.clone();
        async move {
            sqlx::query("UPDATE maidan_notifications SET created_at = $1 WHERE id = $2")
                .bind(aged)
                .bind(id)
                .execute(&pool)
                .await
                .expect("age");
        }
    })
    .await;
}
