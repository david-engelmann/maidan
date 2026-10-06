//! A new holder never inherits an old lease deadline.
//!
//! `claim_next_thread` with a lease writes `assignment_expires_at`; a release
//! used to leave it behind, and `assign_thread` and `claim_thread` did not
//! reset it. The next holder then carried a deadline it never asked for, and
//! once it passed, `claim_next_thread` took the thread away and reported
//! `ClaimExpired` for a holder that had no lease. Found by the claim TLA+
//! spec (`specs/tla/Claim.tla`). On both backends.

use std::time::Duration;

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, EventKind, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    ThreadId,
};
use sqlx::sqlite::SqlitePoolOptions;

const LEASE_SECS: i64 = 1;

struct World {
    channel: ChannelId,
    thread: ThreadId,
    leaser: MemberId,
    holder: MemberId,
    claimer: MemberId,
}

async fn world(store: &dyn Store) -> World {
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
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
        })
        .await
        .unwrap();
    let mut members = Vec::new();
    for handle in ["leaser", "holder", "claimer"] {
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
    World {
        channel: channel.id,
        thread: thread.id,
        leaser: members[0],
        holder: members[1],
        claimer: members[2],
    }
}

#[derive(Clone, Copy, Debug)]
enum Handover {
    /// The leaser releases, then an operator assigns the thread.
    ReleaseThenAssign,
    /// The leaser releases, then a member claims the thread by id.
    ReleaseThenClaim,
    /// An operator reassigns the thread while the lease is live.
    AssignOverLease,
}

async fn the_new_holder_keeps_the_thread(store: &dyn Store, handover: Handover) {
    let w = world(store).await;
    let leased = store
        .claim_next_thread(w.channel, w.leaser, Some(LEASE_SECS))
        .await
        .unwrap()
        .expect("the thread is claimable");
    let token = leased.claim_lease_id.expect("a claim carries a token");

    let held = match handover {
        Handover::ReleaseThenAssign => {
            let released = store
                .release_claim(w.thread, w.leaser, token)
                .await
                .unwrap();
            assert_eq!(released.assignment_expires_at, None, "{handover:?}");
            store.assign_thread(w.thread, w.holder).await.unwrap()
        }
        Handover::ReleaseThenClaim => {
            store
                .release_claim(w.thread, w.leaser, token)
                .await
                .unwrap();
            let claimed = store.claim_thread(w.thread, w.holder).await.unwrap();
            assert!(claimed.claimed, "{handover:?}");
            claimed.thread
        }
        Handover::AssignOverLease => store.assign_thread(w.thread, w.holder).await.unwrap(),
    };
    assert_eq!(held.assignee_id, Some(w.holder), "{handover:?}");
    assert_eq!(held.assignment_expires_at, None, "{handover:?}: no lease");

    // Well past the old lease.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let (taken, events) = store
        .claim_next_thread_with_event(w.channel, w.claimer, Some(LEASE_SECS))
        .await
        .unwrap();
    assert!(
        taken.is_none(),
        "{handover:?}: a holder without a lease lost the thread"
    );
    assert!(
        !events.iter().any(|e| e.kind == EventKind::ClaimExpired),
        "{handover:?}"
    );
    let now = store.get_thread(w.thread).await.unwrap();
    assert_eq!(now.assignee_id, Some(w.holder), "{handover:?}");
}

async fn run_all(store: &dyn Store) {
    for handover in [
        Handover::ReleaseThenAssign,
        Handover::ReleaseThenClaim,
        Handover::AssignOverLease,
    ] {
        the_new_holder_keeps_the_thread(store, handover).await;
    }
}

#[tokio::test]
async fn a_new_holder_never_inherits_a_lease_deadline_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_all(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn a_new_holder_never_inherits_a_lease_deadline_postgres() {
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
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_all(&PostgresStore::for_tests(pool)).await;
}
