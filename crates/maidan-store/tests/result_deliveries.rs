//! Result-delivery state (Cluster 379.1), both backends.
//!
//! The arming upsert is the whole point of this table, so most of this suite is
//! that one predicate: **arm iff this revision is strictly newer than anything
//! this row has seen.** Every case below is a real scenario the delivery step
//! will hit, not a permutation for its own sake:
//!
//! - a first delivery arms;
//! - a second replica reporting the same event does not (the dedup — without it
//!   a 3-replica deploy delivers three times);
//! - a re-review with a newer result does (that is idempotent update-in-place);
//! - a replayed event does not;
//! - an out-of-order older result does not overwrite a newer delivery;
//! - a newer result arriving while a delivery is still pending does arm, because
//!   dropping it would silently lose a review.

use chrono::{DateTime, Duration, SubsecRound, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    status, EgressTarget, NewChannel, NewThread, NewWorkspace, ResultDelivery, ThreadId,
};
use sqlx::sqlite::SqlitePoolOptions;

/// A revision at the precision the schema actually stores.
///
/// Postgres `TIMESTAMPTZ` keeps **microseconds**, so a nanosecond-precision
/// `Utc::now()` does not survive a round trip — asserting that it does is
/// asserting a precision the column never promised. SQLite stores rfc3339 text
/// and *does* keep the nanos, which is why only the Postgres arm noticed. Both
/// arms now use a revision the schema can represent exactly, so the assertions
/// below stay exact equality rather than being weakened to a tolerance.
///
/// This is not a hazard for the arming predicate in production: a real caller
/// passes `ThreadResult::produced_at`, which it read back from this same store at
/// this same precision, and the comparison itself happens in SQL against a bound
/// parameter the backend truncates identically.
fn revision() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

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

fn github() -> EgressTarget {
    EgressTarget::Github {
        repo: "beatgig/bgv3".into(),
        issue_number: 3915,
    }
}

fn slack() -> EgressTarget {
    EgressTarget::Slack {
        channel_id: "C0123ABCDEF".into(),
    }
}

/// A fresh thread, so each scenario starts from "no delivery has ever happened".
async fn thread(store: &dyn Store, name: &str) -> ThreadId {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("ws");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some(name.into()),
        })
        .await
        .expect("thread")
        .id
}

fn assert_pending(d: &ResultDelivery, target: &EgressTarget) {
    assert_eq!(d.status, status::PENDING);
    assert_eq!(d.surface, target.surface().as_str());
    assert_eq!(d.selector, target.selector());
    assert_eq!(d.target().as_ref(), Some(target), "the stored pair decodes");
    assert_eq!(d.attempts, 0);
    assert!(d.delivered_revision.is_none());
    assert!(d.last_error.is_none());
}

async fn run_arming_suite(store: &dyn Store) {
    let tid = thread(store, "arming").await;
    let target = github();
    let r1 = revision();

    // Nothing aimed at this target yet.
    assert!(store
        .get_result_delivery(tid, &target)
        .await
        .expect("get")
        .is_none());

    // The first arm wins and owns the delivery.
    let armed = store
        .arm_result_delivery(tid, &target, r1)
        .await
        .expect("arm")
        .expect("the first caller wins");
    assert_pending(&armed, &target);
    assert_eq!(armed.thread_id, tid);

    // A second replica handling the *same* event loses. This is the dedup: every
    // replica runs the router, so without it the comment is posted N times.
    assert!(
        store
            .arm_result_delivery(tid, &target, r1)
            .await
            .expect("arm again")
            .is_none(),
        "a second replica's arm of the same revision is deduped"
    );

    // A *newer* result while the first is still pending must arm — dropping it
    // would silently lose a review that a human asked for.
    let r2 = r1 + Duration::seconds(1);
    let rearmed = store
        .arm_result_delivery(tid, &target, r2)
        .await
        .expect("arm newer")
        .expect("a newer revision re-arms even while pending");
    assert_eq!(rearmed.id, armed.id, "same row, updated in place");
    assert_eq!(rearmed.armed_revision, r2);

    // Deliver it, recording the handle that makes the next one an edit.
    store
        .mark_result_delivered(armed.id, Some("998877"), r2)
        .await
        .expect("delivered");
    let delivered = store
        .get_result_delivery(tid, &target)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(delivered.status, status::DELIVERED);
    assert_eq!(delivered.delivered_revision, Some(r2));
    assert_eq!(delivered.armed_revision, r2);
    assert_eq!(delivered.attempts, 1);
    assert_eq!(
        delivered.reference(),
        Some(maidan_types::ExternalRef::Github {
            repo: "beatgig/bgv3".into(),
            comment_id: 998877
        }),
        "the stored handle rebuilds into the ref the sender will edit"
    );

    // A replayed event is a no-op.
    assert!(
        store
            .arm_result_delivery(tid, &target, r2)
            .await
            .expect("replay")
            .is_none(),
        "replaying the delivered revision changes nothing"
    );

    // An out-of-order older result must not overwrite a newer delivery.
    assert!(
        store
            .arm_result_delivery(tid, &target, r1)
            .await
            .expect("older")
            .is_none(),
        "an older revision never re-arms"
    );
    let untouched = store
        .get_result_delivery(tid, &target)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(untouched.status, status::DELIVERED);
    assert_eq!(untouched.delivered_revision, Some(r2));

    // A genuine re-review arms again and keeps the handle, which is what turns
    // the next send into an edit rather than a second comment.
    let r3 = r2 + Duration::seconds(1);
    let re_review = store
        .arm_result_delivery(tid, &target, r3)
        .await
        .expect("re-review")
        .expect("a newer revision re-arms");
    assert_eq!(re_review.status, status::PENDING);
    assert_eq!(
        re_review.attempts, 0,
        "a fresh attempt count for the re-send"
    );
    assert_eq!(
        re_review.external_ref.as_deref(),
        Some("998877"),
        "the handle survives re-arming — this is idempotent update-in-place"
    );
    assert_eq!(
        re_review.delivered_revision,
        Some(r2),
        "what landed is still what landed until the new one does"
    );
}

async fn run_disposition_suite(store: &dyn Store) {
    let tid = thread(store, "dispositions").await;
    let rev = revision();

    // A failure leaves the previous delivery's identity intact: whatever is on
    // the surface is still there and still editable.
    let gh = store
        .arm_result_delivery(tid, &github(), rev)
        .await
        .expect("arm")
        .expect("armed");
    store
        .mark_result_delivered(gh.id, Some("998877"), rev)
        .await
        .expect("delivered");
    let rev2 = rev + Duration::seconds(1);
    store
        .arm_result_delivery(tid, &github(), rev2)
        .await
        .expect("arm2")
        .expect("armed2");
    store
        .mark_result_delivery_failed(gh.id, "github api error: status 502")
        .await
        .expect("failed");
    let failed = store
        .get_result_delivery(tid, &github())
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(failed.status, status::FAILED);
    assert_eq!(
        failed.last_error.as_deref(),
        Some("github api error: status 502")
    );
    assert_eq!(
        failed.external_ref.as_deref(),
        Some("998877"),
        "a failed re-send does not forget the object it delivered before"
    );
    assert_eq!(
        failed.delivered_revision,
        Some(rev),
        "and does not claim the new revision landed"
    );
    // A failed row is still re-armable by a newer result, so a fixed connector
    // plus a re-review recovers without operator surgery.
    let rev3 = rev2 + Duration::seconds(1);
    assert!(store
        .arm_result_delivery(tid, &github(), rev3)
        .await
        .expect("arm3")
        .is_some());

    // A skip is a recorded normal outcome, not an error: the producer reads the
    // reason back rather than wondering why nothing arrived.
    let sl = store
        .arm_result_delivery(tid, &slack(), rev)
        .await
        .expect("arm slack")
        .expect("armed slack");
    store
        .mark_result_delivery_skipped(sl.id, "target not in the workspace egress allowlist")
        .await
        .expect("skipped");
    let skipped = store
        .get_result_delivery(tid, &slack())
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(skipped.status, status::SKIPPED);
    assert_eq!(
        skipped.last_error.as_deref(),
        Some("target not in the workspace egress allowlist")
    );
    assert!(
        skipped.delivered_revision.is_none(),
        "a skip never claims a delivery"
    );

    // Both targets are listed for the delivery-status API, stably ordered.
    let listed = store.list_result_deliveries(tid).await.expect("list");
    assert_eq!(listed.len(), 2, "one row per target, never appended to");
    assert_eq!(
        listed
            .iter()
            .map(|d| d.surface.as_str())
            .collect::<Vec<_>>(),
        vec!["github", "slack"]
    );

    // Targets are per thread: another thread's deliveries are not visible here.
    let other = thread(store, "dispositions-other").await;
    assert!(store
        .list_result_deliveries(other)
        .await
        .expect("list other")
        .is_empty());
    assert!(store
        .get_result_delivery(other, &github())
        .await
        .expect("get other")
        .is_none());
}

/// Two targets on one thread are independent deliveries — partial delivery is the
/// model, so one target failing must not disturb the other.
async fn run_per_target_suite(store: &dyn Store) {
    let tid = thread(store, "per-target").await;
    let rev = revision();

    let gh = store
        .arm_result_delivery(tid, &github(), rev)
        .await
        .expect("arm gh")
        .expect("armed gh");
    let sl = store
        .arm_result_delivery(tid, &slack(), rev)
        .await
        .expect("arm slack")
        .expect("armed slack");
    assert_ne!(gh.id, sl.id, "a target gets its own row");

    store
        .mark_result_delivered(sl.id, Some("1699999999.001200"), rev)
        .await
        .expect("slack delivered");
    store
        .mark_result_delivery_failed(gh.id, "boom")
        .await
        .expect("github failed");

    let gh_row = store
        .get_result_delivery(tid, &github())
        .await
        .expect("get gh")
        .expect("exists");
    let sl_row = store
        .get_result_delivery(tid, &slack())
        .await
        .expect("get slack")
        .expect("exists");
    assert_eq!(gh_row.status, status::FAILED);
    assert_eq!(sl_row.status, status::DELIVERED);
    assert_eq!(
        sl_row.reference(),
        Some(maidan_types::ExternalRef::Slack {
            channel_id: "C0123ABCDEF".into(),
            ts: "1699999999.001200".into()
        })
    );
}

/// An unroutable destination still gets a row. The skip is the recorded
/// warning the producer reads; vanishing would look like "we lost it".
async fn run_unroutable_suite(store: &dyn Store) {
    let tid = thread(store, "unroutable").await;
    let rev = revision();
    let row = store
        .arm_unroutable_result_delivery(tid, "discord", "", rev)
        .await
        .expect("arm")
        .expect("armed");
    assert_eq!(row.surface, "discord");
    assert_eq!(row.selector, "");
    assert_eq!(row.status, status::PENDING);
    store
        .mark_result_delivery_skipped(row.id, "unknown surface 'discord'")
        .await
        .expect("skipped");
    let listed = store.list_result_deliveries(tid).await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, status::SKIPPED);
    assert_eq!(
        listed[0].last_error.as_deref(),
        Some("unknown surface 'discord'")
    );
    // Same revision from a second replica is still the dedup.
    assert!(
        store
            .arm_unroutable_result_delivery(tid, "discord", "", rev)
            .await
            .expect("arm again")
            .is_none(),
        "unroutable skips are deduped the same way as routable ones"
    );
}

/// A delivery that landed without an addressable handle (Cluster 378.2's
/// `Ok(None)`) is still a delivery — it just posts again next time rather than
/// editing, because the alternative is PATCHing a guess.
async fn run_unaddressable_suite(store: &dyn Store) {
    let tid = thread(store, "unaddressable").await;
    let rev = revision();
    let d = store
        .arm_result_delivery(tid, &github(), rev)
        .await
        .expect("arm")
        .expect("armed");
    store
        .mark_result_delivered(d.id, None, rev)
        .await
        .expect("delivered");
    let row = store
        .get_result_delivery(tid, &github())
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(row.status, status::DELIVERED);
    assert_eq!(row.delivered_revision, Some(rev));
    assert!(row.external_ref.is_none());
    assert_eq!(
        row.reference(),
        None,
        "no handle means the next revision posts instead of editing"
    );
}

#[tokio::test]
async fn result_deliveries_arm_dedup_and_record_dispositions_sqlite() {
    let store = sqlite().await;
    run_arming_suite(&store).await;
    run_disposition_suite(&store).await;
    run_per_target_suite(&store).await;
    run_unaddressable_suite(&store).await;
    run_unroutable_suite(&store).await;
}

#[tokio::test]
async fn result_deliveries_arm_dedup_and_record_dispositions_postgres() {
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
    run_arming_suite(&store).await;
    run_disposition_suite(&store).await;
    run_per_target_suite(&store).await;
    run_unaddressable_suite(&store).await;
    run_unroutable_suite(&store).await;
}
