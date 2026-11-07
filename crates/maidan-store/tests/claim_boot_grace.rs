//! The boot grace on lapsed claims, on both backends.
//!
//! A server restart outlasts a short lease, so every holder misses its
//! heartbeats while the server is down. During the grace no lease counts as
//! lapsed: `reap_expired_claims` frees none, `claim_next` takes none over and
//! the queue counts agree, while the holder that comes back renews by its
//! lease id and delivers its result. When the grace ends, a lease nobody
//! renewed is reaped as before. Without a grace nothing changes.

use std::sync::Arc;

use chrono::Utc;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, EventKind, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    Thread, WorkspaceId,
};

struct Queue {
    workspace: WorkspaceId,
    channel: ChannelId,
    holder: MemberId,
    next: MemberId,
}

async fn setup(store: &dyn Store, threads: usize) -> Queue {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    for i in 0..threads {
        store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some(format!("task {i}")),
                description: None,
                created_by: None,
            })
            .await
            .unwrap();
    }
    let mut members = Vec::new();
    for handle in ["holder", "next"] {
        let m = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        members.push(m.id);
    }
    Queue {
        workspace: ws.id,
        channel: channel.id,
        holder: members[0],
        next: members[1],
    }
}

/// Claim `count` threads on a live lease, then move each deadline `secs_ago`
/// into the past: holders that could not reach the server to renew. All are
/// claimed before any lapses, since a lapsed lease is claimable and the next
/// claim would take it again.
async fn lapsed(store: &dyn Store, q: &Queue, count: usize, secs_ago: i64) -> Vec<Thread> {
    let mut held = Vec::new();
    for _ in 0..count {
        held.push(
            store
                .claim_next_thread(q.channel, q.holder, Some(300), None)
                .await
                .unwrap()
                .expect("a claimable thread"),
        );
    }
    let mut lapsed = Vec::new();
    for t in held {
        lapsed.push(
            store
                .renew_claim(t.id, q.holder, t.claim_lease_id.unwrap(), -secs_ago)
                .await
                .unwrap(),
        );
    }
    lapsed
}

fn start_grace(store: &dyn Store, secs: i64) {
    store
        .claim_grace()
        .hold_until(Utc::now() + chrono::Duration::seconds(secs));
}

fn end_grace(store: &dyn Store) {
    store
        .claim_grace()
        .hold_until(Utc::now() - chrono::Duration::seconds(1));
}

async fn nothing_for_the_next_claimer(store: &dyn Store, q: &Queue) {
    let (taken, events) = store
        .claim_next_thread_with_event(q.channel, q.next, Some(300), None, None, None)
        .await
        .unwrap();
    assert_eq!(taken.map(|t| t.id), None, "a held lease is not taken over");
    assert!(events.is_empty());
    let (taken, _) = store
        .claim_next_workspace_thread_with_event(q.workspace, q.next, Some(300), None, None, None)
        .await
        .unwrap();
    assert_eq!(taken.map(|t| t.id), None, "nor across the workspace");
    assert_eq!(
        store
            .claim_next_thread(q.channel, q.next, Some(300), None)
            .await
            .unwrap()
            .map(|t| t.id),
        None,
        "nor by the plain claim"
    );
}

/// A lease that lapsed while the server was down is not reaped or taken
/// over during the grace, and its holder renews it and delivers; a lease
/// nobody renews is reaped once the grace ends.
async fn a_lapsed_lease_is_held_through_the_grace_and_reaped_after_it(store: &dyn Store) {
    let q = setup(store, 2).await;
    let held = lapsed(store, &q, 2, 120).await;
    let (back, gone) = (&held[0], &held[1]);
    start_grace(store, 600);

    assert!(
        store
            .reap_expired_claims(Utc::now(), 100)
            .await
            .unwrap()
            .is_empty(),
        "nothing is reaped during the grace"
    );
    for thread in [back, gone] {
        let current = store.get_thread(thread.id).await.unwrap();
        assert_eq!(current.assignee_id, Some(q.holder), "the holder keeps it");
        assert_eq!(current.claim_lease_id, thread.claim_lease_id);
    }
    nothing_for_the_next_claimer(store, &q).await;
    let depth = store.channel_queue_depth(q.channel, None).await.unwrap();
    assert_eq!(
        (depth.ready, depth.assigned),
        (0, 2),
        "the counts agree with claim_next"
    );

    // The holder of `back` reaches the server again: its lapsed lease renews
    // by its lease id, and its result is stored.
    let lease = back.claim_lease_id.unwrap();
    let renewed = store
        .renew_claim(back.id, q.holder, lease, 300)
        .await
        .expect("a lapsed lease renews during the grace");
    assert!(renewed.assignment_expires_at.unwrap() > Utc::now());
    assert_eq!(renewed.claim_lease_id, Some(lease), "the same claim");
    store
        .set_thread_result(
            back.id,
            q.holder,
            lease,
            &serde_json::json!({ "kind": "test.done" }),
        )
        .await
        .expect("the result is not refused with ClaimLost");

    // The grace ends: the lease nobody renewed is reaped, the renewed one is
    // live and left alone.
    end_grace(store);
    let events = store.reap_expired_claims(Utc::now(), 100).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| (e.kind, e.thread_id))
            .collect::<Vec<_>>(),
        vec![(EventKind::ClaimExpired, Some(gone.id))]
    );
    assert_eq!(store.get_thread(gone.id).await.unwrap().assignee_id, None);
    assert_eq!(
        store.get_thread(back.id).await.unwrap().assignee_id,
        Some(q.holder)
    );
}

/// When the grace ends, `claim_next` takes a lapsed lease over again, with
/// the dead holder's `ClaimExpired`.
async fn claim_next_takes_a_lapsed_lease_over_once_the_grace_ends(store: &dyn Store) {
    let q = setup(store, 1).await;
    let held = lapsed(store, &q, 1, 120).await.remove(0);
    start_grace(store, 600);
    nothing_for_the_next_claimer(store, &q).await;

    end_grace(store);
    let (taken, events) = store
        .claim_next_thread_with_event(q.channel, q.next, Some(300), None, None, None)
        .await
        .unwrap();
    assert_eq!(taken.map(|t| t.id), Some(held.id));
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::ClaimExpired, EventKind::ThreadAssignmentChanged]
    );
}

/// With no grace, as with `MAIDAN_CLAIM_REAPER_BOOT_GRACE_SECS=0`, a lapsed
/// lease is reaped at once.
async fn without_a_grace_a_lapsed_lease_is_reaped_at_once(store: &dyn Store) {
    let q = setup(store, 1).await;
    let held = lapsed(store, &q, 1, 5).await.remove(0);
    assert!(!store.claim_grace().holding(Utc::now()));
    let events = store.reap_expired_claims(Utc::now(), 100).await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| (e.kind, e.thread_id))
            .collect::<Vec<_>>(),
        vec![(EventKind::ClaimExpired, Some(held.id))]
    );
}

async fn run_suite(store: Arc<dyn Store>) {
    // First, before any grace is set on this store.
    without_a_grace_a_lapsed_lease_is_reaped_at_once(store.as_ref()).await;
    a_lapsed_lease_is_held_through_the_grace_and_reaped_after_it(store.as_ref()).await;
    claim_next_takes_a_lapsed_lease_over_once_the_grace_ends(store.as_ref()).await;
}

#[tokio::test]
async fn lapsed_leases_wait_out_the_boot_grace_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(Arc::new(SqliteStore::for_tests(pool))).await;
}

#[tokio::test]
async fn lapsed_leases_wait_out_the_boot_grace_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(Arc::new(PostgresStore::for_tests(pool))).await;
}
