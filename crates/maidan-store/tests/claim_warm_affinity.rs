//! `claim_next` hands a thread back to the agent that last held it ahead of
//! older work of the same aged priority (Context Economics C6): that agent's
//! prompt prefix is the one most likely still cached. It is a tiebreak only.
//! Both backends, both claim paths.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, Thread};
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
    SqliteStore::for_tests(pool)
}

async fn claim(
    store: &dyn Store,
    channel: maidan_types::ChannelId,
    member: maidan_types::MemberId,
    with_event: bool,
) -> Option<Thread> {
    if with_event {
        store
            .claim_next_thread_with_event(channel, member, Some(60), None, None)
            .await
            .expect("claim")
            .0
    } else {
        store
            .claim_next_thread(channel, member, Some(60), None)
            .await
            .expect("claim")
    }
}

async fn run_suite(store: &dyn Store) {
    for with_event in [false, true] {
        let ws = store
            .create_workspace(NewWorkspace {
                name: "warm".into(),
            })
            .await
            .expect("ws");
        let mut members = Vec::new();
        for handle in ["alice", "bob"] {
            members.push(
                store
                    .create_member(NewMember {
                        workspace_id: ws.id,
                        handle: handle.into(),
                        display_name: None,
                        kind: MemberKind::Agent,
                    })
                    .await
                    .expect("member"),
            );
        }
        let (alice, bob) = (&members[0], &members[1]);
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "warm".into(),
                topic: None,
                private: false,
            })
            .await
            .expect("channel");
        let mut threads = Vec::new();
        for title in ["older", "newer"] {
            threads.push(
                store
                    .create_thread(NewThread {
                        channel_id: channel.id,
                        parent_thread_id: None,
                        title: Some(title.into()),
                        description: None,
                        created_by: None,
                    })
                    .await
                    .expect("thread"),
            );
        }

        // Oldest first: alice gets the older thread, bob the newer one.
        let a = claim(store, channel.id, alice.id, with_event)
            .await
            .expect("a");
        assert_eq!(a.id, threads[0].id);
        let b = claim(store, channel.id, bob.id, with_event)
            .await
            .expect("b");
        assert_eq!(b.id, threads[1].id);
        for (thread, member) in [(&a, alice), (&b, bob)] {
            store
                .release_claim(thread.id, member.id, thread.claim_lease_id.expect("leased"))
                .await
                .expect("release");
        }

        // Both are back in the queue. Bob claims first and gets the newer
        // thread he last held, not the older one; alice gets hers.
        let b = claim(store, channel.id, bob.id, with_event)
            .await
            .expect("b again");
        assert_eq!(
            b.id, threads[1].id,
            "bob's warm thread should come back to him"
        );
        let a = claim(store, channel.id, alice.id, with_event)
            .await
            .expect("a again");
        assert_eq!(a.id, threads[0].id);
    }
}

#[tokio::test]
async fn claim_next_prefers_the_thread_the_claimer_last_held_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn claim_next_prefers_the_thread_the_claimer_last_held_postgres() {
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
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
