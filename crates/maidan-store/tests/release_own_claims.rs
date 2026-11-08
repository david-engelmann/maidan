//! A claimant that restarted frees every live claim its member holds in one
//! workspace at once (`release_member_claims_with_events`), without the
//! fencing tokens the old process took with it.
//!
//! Only the member's own live leased claims in that workspace are released:
//! another member's claim, a lease-less assignment, a lapsed lease (the
//! reaper's) and the member's claims in another workspace are left alone.
//! Each release is the one `release_claim` makes, so the old process is fenced
//! out afterwards. On both backends.

use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    ChannelId, ChannelMemberRole, EventKind, MemberId, MemberKind, NewChannel, NewMember,
    NewThread, NewWorkspace, ThreadId, WorkspaceId,
};

const LEASE_SECS: i64 = 600;
const NOTE: &str = "the claimant restarted";

async fn workspace(store: &dyn Store, name: &str) -> (WorkspaceId, ChannelId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
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
    (ws.id, channel.id)
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id
}

async fn thread(store: &dyn Store, channel: ChannelId, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id
}

async fn a_restarted_claimant_releases_only_its_own_live_claims(store: &dyn Store) {
    let (ws, channel) = workspace(store, "w").await;
    let (other_ws, other_channel) = workspace(store, "w2").await;
    let a = member(store, ws, "a").await;
    let b = member(store, ws, "b").await;
    let a_elsewhere = member(store, other_ws, "a-elsewhere").await;

    // Created in this order so each claim_next takes the thread named for it.
    let first = thread(store, channel, "a's first").await;
    let second = thread(store, channel, "a's second").await;
    let bs = thread(store, channel, "b's").await;
    let assigned = thread(store, channel, "assigned to a").await;
    let lapsing = thread(store, channel, "a's lapsing").await;
    let elsewhere = thread(store, other_channel, "a-elsewhere's").await;

    let claim = |channel: ChannelId, who: MemberId, lease: i64| async move {
        store
            .claim_next_thread(channel, who, Some(lease), None)
            .await
            .unwrap()
            .expect("a thread is claimable")
    };
    let a_first = claim(channel, a, LEASE_SECS).await;
    let a_second = claim(channel, a, LEASE_SECS).await;
    let b_claim = claim(channel, b, LEASE_SECS).await;
    assert_eq!((a_first.id, a_second.id, b_claim.id), (first, second, bs));
    let a_second_lease = a_second.claim_lease_id.unwrap();
    store
        .acknowledge_claim(second, a, a_second_lease)
        .await
        .unwrap();
    store
        .assign_thread_with_event(assigned, a, b, None)
        .await
        .unwrap();
    let a_lapsing = claim(channel, a, 1).await;
    assert_eq!(a_lapsing.id, lapsing);
    let a_elsewhere_claim = claim(other_channel, a_elsewhere, LEASE_SECS).await;
    assert_eq!(a_elsewhere_claim.id, elsewhere);
    // A claim in a private channel a was then removed from: release_claim
    // would refuse it, so the bulk release leaves it to lapse too.
    let private = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "private".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap()
        .id;
    store
        .add_channel_member(private, a, ChannelMemberRole::Member)
        .await
        .unwrap();
    let unreadable = thread(store, private, "a's, unreadable later").await;
    assert_eq!(claim(private, a, LEASE_SECS).await.id, unreadable);
    store.remove_channel_member(private, a).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // The member's claims are all in its own workspace, so naming another
    // workspace releases nothing.
    assert!(store
        .release_member_claims_with_events(other_ws, a, Some(NOTE.into()))
        .await
        .unwrap()
        .is_empty());

    let released = store
        .release_member_claims_with_events(ws, a, Some(NOTE.into()))
        .await
        .unwrap();
    let ids: Vec<ThreadId> = released.iter().map(|(t, _)| t.id).collect();
    let mut expected = vec![first, second];
    expected.sort_by_key(|id| id.0);
    assert_eq!(ids, expected, "exactly a's two live claims in w");
    for (thread, stored) in &released {
        assert_eq!(thread.assignee_id, None);
        assert_eq!(thread.claim_lease_id, None);
        assert_eq!(thread.assignment_expires_at, None);
        assert_eq!(thread.work_started_at, None, "the working clock is cleared");
        assert_eq!(stored.kind, EventKind::ThreadAssignmentChanged);
        assert_eq!(stored.thread_id, Some(thread.id));
        assert!(
            stored.payload.to_string().contains(NOTE),
            "the event carries the note: {}",
            stored.payload
        );
    }

    let held = |id: ThreadId| async move { store.get_thread(id).await.unwrap().assignee_id };
    assert!(
        store
            .get_thread_budget(second)
            .await
            .unwrap()
            .map_or(0, |b| b.used_wall_secs)
            >= 1,
        "the acknowledged claim's worked time is charged"
    );
    assert_eq!(held(bs).await, Some(b), "another member's claim is kept");
    assert_eq!(
        held(unreadable).await,
        Some(a),
        "a claim on a thread a can no longer read is kept"
    );
    assert_eq!(
        held(assigned).await,
        Some(a),
        "a lease-less assignment is kept"
    );
    assert_eq!(
        held(lapsing).await,
        Some(a),
        "a lapsed lease is the reaper's"
    );
    assert_eq!(
        held(elsewhere).await,
        Some(a_elsewhere),
        "another workspace's claim is kept"
    );

    // Idempotent: nothing live is left to release.
    assert!(store
        .release_member_claims_with_events(ws, a, None)
        .await
        .unwrap()
        .is_empty());

    // The old process is fenced out of what it held.
    assert!(matches!(
        store.renew_claim(second, a, a_second_lease, 60).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store
            .release_claim(first, a, a_first.claim_lease_id.unwrap())
            .await,
        Err(StoreError::NotFound)
    ));
}

async fn a_claim_that_changed_hands_is_not_released(store: &dyn Store) {
    let (ws, channel) = workspace(store, "w").await;
    let a = member(store, ws, "a").await;
    let b = member(store, ws, "b").await;
    let only = thread(store, channel, "one task").await;
    let a_claim = store
        .claim_next_thread(channel, a, Some(1), None)
        .await
        .unwrap()
        .expect("claimable");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // The lapsed claim is taken over by b before a's restart gets to it.
    let (b_claim, _) = store
        .claim_next_thread_with_event(channel, b, Some(LEASE_SECS), None, None, None)
        .await
        .unwrap();
    let b_claim = b_claim.expect("the lapsed claim is claimable");
    assert_eq!((a_claim.id, b_claim.id), (only, only));

    assert!(store
        .release_member_claims_with_events(ws, a, None)
        .await
        .unwrap()
        .is_empty());
    let thread = store.get_thread(only).await.unwrap();
    assert_eq!(thread.assignee_id, Some(b));
    assert_eq!(thread.claim_lease_id, b_claim.claim_lease_id);
}

async fn run_suite(store: &dyn Store) {
    a_restarted_claimant_releases_only_its_own_live_claims(store).await;
    a_claim_that_changed_hands_is_not_released(store).await;
}

#[tokio::test]
async fn a_restarted_claimant_releases_its_own_claims_sqlite() {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    run_suite(store.as_ref()).await;
}

#[tokio::test]
async fn a_restarted_claimant_releases_its_own_claims_postgres() {
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
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool));
    run_suite(store.as_ref()).await;
}
