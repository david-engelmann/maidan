//! Assignment read-side (Cluster 190): list-mine is scoped to the member;
//! claim-next atomically takes the oldest unassigned thread.

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
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

async fn run_readside_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "aq".into() })
        .await
        .expect("ws");
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("agent");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mk_thread = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };
    let first = store.create_thread(mk_thread("first")).await.expect("t1");
    let second = store.create_thread(mk_thread("second")).await.expect("t2");

    // Nothing assigned yet.
    assert!(store
        .list_assigned_threads(ws.id, agent.id)
        .await
        .expect("list0")
        .is_empty());

    // claim-next takes the OLDEST unassigned thread (first).
    let claimed = store
        .claim_next_thread(channel.id, agent.id, None)
        .await
        .expect("claim1")
        .expect("some work");
    assert_eq!(claimed.id, first.id, "oldest unassigned is claimed first");
    assert_eq!(claimed.assignee_id, Some(agent.id));

    // Now it's in the agent's queue.
    let queue = store
        .list_assigned_threads(ws.id, agent.id)
        .await
        .expect("list1");
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].id, first.id);

    // A second claim takes the next oldest (second); a third finds nothing.
    let claimed2 = store
        .claim_next_thread(channel.id, agent.id, None)
        .await
        .expect("claim2")
        .expect("more work");
    assert_eq!(claimed2.id, second.id);
    assert!(
        store
            .claim_next_thread(channel.id, agent.id, None)
            .await
            .expect("claim3")
            .is_none(),
        "no unassigned work left"
    );

    // list-mine is member-scoped: a different member's queue is empty.
    let other = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "other".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("other");
    assert!(store
        .list_assigned_threads(ws.id, other.id)
        .await
        .expect("list-other")
        .is_empty());

    // --- leases (Cluster 192): an expired lease is reclaimable; the holder can
    // renew; a non-holder cannot ---
    let lease_ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "lease".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("lease-ch");
    let m1 = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "lease-a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("m1");
    let m2 = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "lease-b".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("m2");
    let leased = store
        .create_thread(NewThread {
            channel_id: lease_ch.id,
            parent_thread_id: None,
            title: Some("leased".into()),
        })
        .await
        .expect("leased-thread");

    // m1 claims with an already-past lease (dead agent).
    let c1 = store
        .claim_next_thread(lease_ch.id, m1.id, Some(-1))
        .await
        .expect("claim m1")
        .expect("some");
    assert_eq!(c1.id, leased.id);
    assert_eq!(c1.assignee_id, Some(m1.id));
    // m2 reclaims the expired lease.
    let c2 = store
        .claim_next_thread(lease_ch.id, m2.id, Some(3600))
        .await
        .expect("claim m2")
        .expect("reclaimed");
    assert_eq!(c2.id, leased.id, "same thread reclaimed");
    assert_eq!(
        c2.assignee_id,
        Some(m2.id),
        "expired lease reclaimed by the next claimer"
    );
    // Lease is now valid → nothing else claimable.
    assert!(store
        .claim_next_thread(lease_ch.id, m1.id, Some(3600))
        .await
        .expect("claim none")
        .is_none());
    // The holder renews; a non-holder is rejected.
    store
        .renew_claim(leased.id, m2.id, 7200)
        .await
        .expect("holder renews");
    assert!(matches!(
        store.renew_claim(leased.id, m1.id, 7200).await,
        Err(maidan_store::StoreError::NotFound)
    ));
}

#[tokio::test]
async fn assignment_readside_lists_mine_and_claims_oldest_sqlite() {
    let store = sqlite().await;
    run_readside_suite(&store).await;
}

#[tokio::test]
async fn assignment_readside_lists_mine_and_claims_oldest_postgres() {
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
    run_readside_suite(&store).await;
}
