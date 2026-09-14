//! Explicit dispatch-block store (Cluster 386, Wave 2 #27): set/clear/get +
//! channel list over the closed `BlockedReason` enum, `claim_next` skip
//! (386.2), and `BlockedResolved` on clear (386.3). Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    BlockedReason, Event, EventKind, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
};
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

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mk = |title: &str| {
        let cid = channel.id;
        let title = title.to_string();
        async move {
            store
                .create_thread(NewThread {
                    channel_id: cid,
                    parent_thread_id: None,
                    title: Some(title),
                })
                .await
                .expect("thread")
        }
    };
    let t1 = mk("t1").await;
    let t2 = mk("t2").await;

    assert!(store.get_thread_block(t1.id).await.expect("get").is_none());
    assert!(store
        .list_blocked_threads(channel.id)
        .await
        .expect("list")
        .is_empty());

    let set = store
        .set_thread_block(t1.id, BlockedReason::Gate, member.id)
        .await
        .expect("set");
    assert_eq!(set.thread_id, t1.id);
    assert_eq!(set.reason, BlockedReason::Gate);
    assert_eq!(set.set_by, member.id);

    let got = store
        .get_thread_block(t1.id)
        .await
        .expect("get")
        .expect("blocked");
    assert_eq!(got.reason, BlockedReason::Gate);

    // Re-set upserts the reason (closed enum, not a free-text park).
    store
        .set_thread_block(t1.id, BlockedReason::Human, member.id)
        .await
        .expect("reset");
    assert_eq!(
        store
            .get_thread_block(t1.id)
            .await
            .expect("get")
            .expect("blocked")
            .reason,
        BlockedReason::Human
    );

    // Every closed reason is persistable.
    for &reason in BlockedReason::ALL {
        store
            .set_thread_block(t2.id, reason, member.id)
            .await
            .expect("set each reason");
        assert_eq!(
            store
                .get_thread_block(t2.id)
                .await
                .expect("get")
                .expect("blocked")
                .reason,
            reason
        );
    }

    let listed = store.list_blocked_threads(channel.id).await.expect("list");
    assert_eq!(listed.len(), 2);

    // Clear returns the row once, then None (idempotent).
    let cleared = store
        .clear_thread_block(t1.id)
        .await
        .expect("clear")
        .expect("had a block");
    assert_eq!(cleared.reason, BlockedReason::Human);
    assert!(store
        .clear_thread_block(t1.id)
        .await
        .expect("clear again")
        .is_none());
    assert!(store.get_thread_block(t1.id).await.expect("get").is_none());
    assert_eq!(
        store
            .list_blocked_threads(channel.id)
            .await
            .expect("list")
            .len(),
        1,
        "only t2 remains blocked"
    );
}

/// claim_next skips an older explicitly-blocked thread; queue-depth `blocked`
/// counts it (alongside DAG-blocked); clearing restores claimability.
/// Distinct from Cluster 218: a `child` reason is not "deps must be terminal".
async fn run_claim_skip_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "bs".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "cs".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    // t1 is older (claim_next would prefer it) but blocked; t2 is claimable.
    let t1 = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t1".into()),
        })
        .await
        .expect("t1");
    let t2 = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t2".into()),
        })
        .await
        .expect("t2");
    // t3 depends on t2 (non-terminal) — DAG-blocked, no explicit reason.
    let t3 = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t3".into()),
        })
        .await
        .expect("t3");
    store
        .add_thread_dependency(t3.id, t2.id)
        .await
        .expect("dag edge");

    store
        .set_thread_block(t1.id, BlockedReason::Child, member.id)
        .await
        .expect("block");

    let depth = store.channel_queue_depth(channel.id).await.expect("depth");
    assert_eq!(depth.open, 3);
    assert_eq!(depth.ready, 1, "only t2 is ready");
    assert_eq!(
        depth.blocked, 2,
        "t1 explicit-block + t3 DAG-deps; distinct reasons, same bucket"
    );
    assert_eq!(depth.unclaimable, 0, "363 park is a different table");

    let claimed = store
        .claim_next_thread(channel.id, member.id, None)
        .await
        .expect("claim_next")
        .expect("claimed something");
    assert_eq!(claimed.id, t2.id, "explicitly blocked t1 is skipped");

    // The with_event path shares the same skip.
    let (again, events) = store
        .claim_next_thread_with_event(channel.id, member.id, None)
        .await
        .expect("claim_next_with_event");
    assert!(again.is_none(), "t1 still blocked, t3 still DAG-blocked");
    assert!(events.is_empty());

    // Unblock t1 → it becomes claimable (t3 still skipped by 218).
    store
        .clear_thread_block(t1.id)
        .await
        .expect("clear")
        .expect("had a block");
    let claimed2 = store
        .claim_next_thread(channel.id, member.id, None)
        .await
        .expect("claim_next")
        .expect("claimed something");
    assert_eq!(claimed2.id, t1.id, "unblocked t1 is now claimable");
}

/// Clearing a block appends `BlockedResolved` atomically; a second clear is a
/// no-op (no event). The event carries the reason that resolved.
async fn run_blocked_resolved_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "br".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "resolver".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "brc".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("blocked".into()),
        })
        .await
        .expect("thread");
    store
        .set_thread_block(thread.id, BlockedReason::Quota, member.id)
        .await
        .expect("block");

    let (cleared, stored) = store
        .clear_thread_block_with_event(thread.id, member.id)
        .await
        .expect("clear_with_event");
    let cleared = cleared.expect("had a block");
    let stored = stored.expect("emitted BlockedResolved");
    assert_eq!(cleared.reason, BlockedReason::Quota);
    assert_eq!(stored.kind, EventKind::BlockedResolved);
    assert_eq!(stored.thread_id, Some(thread.id));
    assert_eq!(stored.workspace_id, Some(ws.id));
    assert_eq!(stored.channel_id, Some(channel.id));
    let event: Event = serde_json::from_value(stored.payload.clone()).expect("payload");
    match event {
        Event::BlockedResolved {
            reason,
            resolved_by,
            thread_id,
            ..
        } => {
            assert_eq!(reason, BlockedReason::Quota);
            assert_eq!(resolved_by, member.id);
            assert_eq!(thread_id, thread.id);
        }
        other => panic!("expected BlockedResolved, got {other:?}"),
    }

    let (again, ev) = store
        .clear_thread_block_with_event(thread.id, member.id)
        .await
        .expect("clear again");
    assert!(again.is_none());
    assert!(ev.is_none(), "idempotent clear must not re-emit");
}

#[tokio::test]
async fn thread_blocks_set_clear_get_list_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
    run_claim_skip_suite(&store).await;
    run_blocked_resolved_suite(&store).await;
}

#[tokio::test]
async fn thread_blocks_set_clear_get_list_postgres() {
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
    run_suite(&store).await;
    run_claim_skip_suite(&store).await;
    run_blocked_resolved_suite(&store).await;
}
