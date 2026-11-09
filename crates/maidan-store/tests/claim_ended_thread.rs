//! An explicit claim of a thread that has ended is refused, and a holder
//! whose lease is dead is told its claim is lost when it acknowledges or
//! releases, as it is when it renews. Identically on both backends.
//!
//! A cancel parks the thread, but a close through review or as abandoned
//! does not, so the park alone let a closed thread be claimed. The store
//! reads the state in the claim's own transaction.

use std::time::Duration;

use chrono::Utc;
use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations, StoreError};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// The raw pool, so the test can delete a thread the way a tombstone does.
enum Pool {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
}

impl Pool {
    async fn tombstone(&self, id: ThreadId) {
        let now = Utc::now();
        match self {
            Pool::Sqlite(p) => {
                sqlx::query("UPDATE maidan_threads SET tombstoned_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(id.0)
                    .execute(p)
                    .await
                    .expect("tombstone");
            }
            Pool::Postgres(p) => {
                sqlx::query("UPDATE maidan_threads SET tombstoned_at = $1 WHERE id = $2")
                    .bind(now)
                    .bind(id.0)
                    .execute(p)
                    .await
                    .expect("tombstone");
            }
        }
    }
}

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    worker: MemberId,
    other: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let mut members = Vec::new();
    for handle in ["worker", "other"] {
        members.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect("member")
                .id,
        );
    }
    Room {
        ws: ws.id,
        channel: channel.id,
        worker: members[0],
        other: members[1],
    }
}

async fn task(store: &dyn Store, room: &Room, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: Some(room.worker),
        })
        .await
        .expect("thread")
        .id
}

async fn walk(store: &dyn Store, room: &Room, thread: ThreadId, actions: &[ThreadAction]) {
    for action in actions {
        store
            .transition_thread(thread, room.worker, *action)
            .await
            .expect("transition");
    }
}

fn audit(ws: WorkspaceId, actor: MemberId, thread: ThreadId) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(ws),
        actor_id: Some(actor),
        action: "thread.close_abandoned".into(),
        target_kind: Some("thread".into()),
        target_id: Some(thread.0),
        metadata: serde_json::json!({}),
    }
}

async fn an_explicit_claim_of_an_ended_thread_is_refused(store: &dyn Store, pool: &Pool) {
    let room = room(store, "ended").await;

    let closed = task(store, &room, "closed through review").await;
    walk(
        store,
        &room,
        closed,
        &[ThreadAction::StartReview, ThreadAction::Close],
    )
    .await;
    // Closed as abandoned: not parked, so only the state can refuse it.
    let abandoned = task(store, &room, "abandoned").await;
    store
        .close_abandoned_thread(
            abandoned,
            room.worker,
            false,
            &CloseAbandoned {
                reason: CloseReason::Abandoned,
                duplicate_of: None,
            },
            audit(room.ws, room.worker, abandoned),
        )
        .await
        .expect("close as abandoned");
    assert!(
        store
            .get_thread_unclaimable(abandoned)
            .await
            .expect("park")
            .is_none(),
        "a close as abandoned leaves no park"
    );
    let cancelled = task(store, &room, "cancelled").await;
    store
        .cancel_thread(
            cancelled,
            room.worker,
            "not needed",
            CancelAuthority::Creator,
        )
        .await
        .expect("cancel");
    let archived = task(store, &room, "archived").await;
    walk(
        store,
        &room,
        archived,
        &[
            ThreadAction::StartReview,
            ThreadAction::Close,
            ThreadAction::Archive,
        ],
    )
    .await;

    for (thread, what) in [
        (closed, "closed"),
        (abandoned, "closed as abandoned"),
        (cancelled, "cancelled"),
        (archived, "archived"),
    ] {
        let plain = store.claim_thread(thread, room.other).await;
        assert!(
            matches!(plain, Err(StoreError::Ended(_))),
            "{what}: {plain:?}"
        );
        let with_event = store.claim_thread_with_event(thread, room.other).await;
        assert!(
            matches!(with_event, Err(StoreError::Ended(_))),
            "{what}: {with_event:?}"
        );
        assert_eq!(
            store.get_thread(thread).await.expect("thread").assignee_id,
            None,
            "{what}: nothing was claimed"
        );
    }

    // How each one ended, as the egress worker reads it before a change
    // delivery: only a cancel and a close as abandoned ended without landing.
    for (thread, how) in [
        (cancelled, Some("cancelled")),
        (abandoned, Some("abandoned")),
        (closed, None),
        (archived, None),
        (ThreadId::new(), None),
    ] {
        assert_eq!(
            store
                .thread_unlanded_close(thread)
                .await
                .expect("unlanded close")
                .as_deref(),
            how,
            "{thread:?}"
        );
    }

    let deleted = task(store, &room, "deleted").await;
    pool.tombstone(deleted).await;
    let plain = store.claim_thread(deleted, room.other).await;
    assert!(matches!(plain, Err(StoreError::Gone(_))), "{plain:?}");
    let with_event = store.claim_thread_with_event(deleted, room.other).await;
    assert!(
        matches!(with_event, Err(StoreError::Gone(_))),
        "{with_event:?}"
    );

    let missing = store.claim_thread(ThreadId::new(), room.other).await;
    assert!(matches!(missing, Err(StoreError::NotFound)), "{missing:?}");

    // Open work and work in review are still claimable.
    let open = task(store, &room, "open").await;
    assert!(
        store
            .claim_thread(open, room.other)
            .await
            .expect("claim an open thread")
            .claimed
    );
    let reviewing = task(store, &room, "in review").await;
    walk(store, &room, reviewing, &[ThreadAction::StartReview]).await;
    let (claimed, event) = store
        .claim_thread_with_event(reviewing, room.other)
        .await
        .expect("claim a thread in review");
    assert!(claimed.claimed);
    assert!(event.is_some(), "a claim that wins records its event");
}

async fn acknowledge_and_release_on_a_dead_lease_answer_claim_lost(store: &dyn Store) {
    let room = room(store, "lease").await;
    let dead = task(store, &room, "lapses").await;
    let live = task(store, &room, "stays live").await;
    // Both claimed on a live lease first: a lapsed lease is claimable, and
    // the second claim would take it over.
    let held = store
        .claim_next_thread(room.channel, room.worker, Some(3600), None)
        .await
        .expect("claim")
        .expect("the oldest thread");
    assert_eq!(held.id, dead);
    let lease = held.claim_lease_id.expect("a claim has a token");
    let kept = store
        .claim_next_thread(room.channel, room.other, Some(3600), None)
        .await
        .expect("claim")
        .expect("the next thread");
    assert_eq!(kept.id, live);

    // The deadline passes; nothing has reaped the claim.
    store
        .renew_claim(dead, room.worker, lease, -5)
        .await
        .expect("move the deadline into the past");

    let ack = store.acknowledge_claim(dead, room.worker, lease).await;
    assert!(matches!(ack, Err(StoreError::ClaimLost(_))), "{ack:?}");
    let release = store.release_claim(dead, room.worker, lease).await;
    assert!(
        matches!(release, Err(StoreError::ClaimLost(_))),
        "{release:?}"
    );
    let with_event = store
        .release_claim_with_event(dead, room.worker, lease, None)
        .await;
    assert!(
        matches!(with_event, Err(StoreError::ClaimLost(_))),
        "{with_event:?}"
    );
    let after = store.get_thread(dead).await.expect("thread");
    assert_eq!(
        after.assignee_id,
        Some(room.worker),
        "no refused release let go of it"
    );
    assert_eq!(after.claim_lease_id, Some(lease));
    assert_eq!(
        after.work_started_at, None,
        "no refused acknowledge started it"
    );

    // A token that never held the thread learns the same.
    let stranger = store
        .acknowledge_claim(dead, room.other, ClaimLeaseId::new())
        .await;
    assert!(
        matches!(stranger, Err(StoreError::ClaimLost(_))),
        "{stranger:?}"
    );

    // A thread that does not exist is still not found.
    let missing = ThreadId::new();
    let ack = store.acknowledge_claim(missing, room.worker, lease).await;
    assert!(matches!(ack, Err(StoreError::NotFound)), "{ack:?}");
    let release = store.release_claim(missing, room.worker, lease).await;
    assert!(matches!(release, Err(StoreError::NotFound)), "{release:?}");
    let with_event = store
        .release_claim_with_event(missing, room.worker, lease, None)
        .await;
    assert!(
        matches!(with_event, Err(StoreError::NotFound)),
        "{with_event:?}"
    );

    // During the boot grace a lapsed lease is held, as a renewal reads it;
    // once the grace ends it is lost again.
    store
        .claim_grace()
        .hold_until(Utc::now() + chrono::Duration::seconds(600));
    store
        .acknowledge_claim(dead, room.worker, lease)
        .await
        .expect("a lapsed lease acknowledges during the grace");
    store
        .claim_grace()
        .hold_until(Utc::now() - chrono::Duration::seconds(1));
    let release = store.release_claim(dead, room.worker, lease).await;
    assert!(
        matches!(release, Err(StoreError::ClaimLost(_))),
        "{release:?}"
    );

    // The live lease still acknowledges and releases.
    let other_lease = kept.claim_lease_id.expect("a claim has a token");
    store
        .acknowledge_claim(live, room.other, other_lease)
        .await
        .expect("a live lease acknowledges");
    let (released, _) = store
        .release_claim_with_event(live, room.other, other_lease, None)
        .await
        .expect("a live lease releases");
    assert_eq!(released.assignee_id, None);
}

async fn run_suite(store: &dyn Store, pool: &Pool) {
    an_explicit_claim_of_an_ended_thread_is_refused(store, pool).await;
    acknowledge_and_release_on_a_dead_lease_answer_claim_lost(store).await;
}

#[tokio::test]
async fn ended_threads_refuse_a_claim_and_dead_leases_are_claim_lost_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    run_suite(&store, &Pool::Sqlite(pool)).await;
}

#[tokio::test]
async fn ended_threads_refuse_a_claim_and_dead_leases_are_claim_lost_postgres() {
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
    run_suite(&store, &Pool::Postgres(pool)).await;
}
