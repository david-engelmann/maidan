//! Subscription/follows (Cluster 244, Arc H): follow/unfollow a channel or thread,
//! list a member's follows, and the router's follower-set queries. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
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

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "follows".into(),
        })
        .await
        .expect("ws");
    let a = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("a");
    let b = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "b".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("b");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .expect("thread");

    // Channel follows.
    store
        .follow_channel(a.id, channel.id)
        .await
        .expect("a follow ch");
    store
        .follow_channel(b.id, channel.id)
        .await
        .expect("b follow ch");
    store
        .follow_channel(a.id, channel.id)
        .await
        .expect("idempotent"); // no-op
    let followers = store
        .channel_followers(channel.id)
        .await
        .expect("followers");
    assert_eq!(followers.len(), 2, "both members follow the channel");
    assert_eq!(
        store
            .list_channel_follows(a.id)
            .await
            .expect("list a")
            .len(),
        1
    );

    // Unfollow.
    assert!(store
        .unfollow_channel(a.id, channel.id)
        .await
        .expect("unfollow"));
    assert!(
        !store
            .unfollow_channel(a.id, channel.id)
            .await
            .expect("unfollow again"),
        "second unfollow removes nothing"
    );
    assert_eq!(
        store.channel_followers(channel.id).await.expect("f").len(),
        1
    );
    assert!(store
        .list_channel_follows(a.id)
        .await
        .expect("list a2")
        .is_empty());

    // Thread follows are independent.
    store
        .follow_thread(a.id, thread.id)
        .await
        .expect("a follow thread");
    let tf = store
        .thread_followers(thread.id)
        .await
        .expect("thread followers");
    assert_eq!(tf, vec![a.id]);
    assert_eq!(
        store
            .list_thread_follows(a.id)
            .await
            .expect("list tf")
            .len(),
        1
    );
    assert!(store
        .unfollow_thread(a.id, thread.id)
        .await
        .expect("unfollow thread"));
    assert!(store
        .thread_followers(thread.id)
        .await
        .expect("tf2")
        .is_empty());

    // Leaf mute (Cluster 356, F7) — independent of follows.
    assert!(!store
        .is_thread_muted(a.id, thread.id)
        .await
        .expect("not muted initially"));
    store.mute_thread(a.id, thread.id).await.expect("a mutes");
    store
        .mute_thread(a.id, thread.id)
        .await
        .expect("mute idempotent");
    store.mute_thread(b.id, thread.id).await.expect("b mutes");
    assert!(store
        .is_thread_muted(a.id, thread.id)
        .await
        .expect("a muted"));
    let mut muters = store.thread_muters(thread.id).await.expect("muters");
    muters.sort_by_key(|m| m.0);
    let mut want = vec![a.id, b.id];
    want.sort_by_key(|m| m.0);
    assert_eq!(muters, want, "both members mute the thread");
    assert!(store
        .unmute_thread(a.id, thread.id)
        .await
        .expect("a unmutes"));
    assert!(!store
        .unmute_thread(a.id, thread.id)
        .await
        .expect("second unmute removes nothing"),);
    assert!(!store
        .is_thread_muted(a.id, thread.id)
        .await
        .expect("a no longer muted"));
    assert_eq!(
        store.thread_muters(thread.id).await.expect("muters2"),
        vec![b.id]
    );
}

#[tokio::test]
async fn follows_channel_and_thread_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn follows_channel_and_thread_postgres() {
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
}
