//! Siblings that share a prompt prefix (the children of one parent thread,
//! such as a recipe's children) warm, then fan out (Context Economics C5).
//! `claim_next` hands out one sibling first and holds the rest back until
//! that claim reports usage, which means the provider has written the shared
//! prefix, so the rest read it instead of each writing it cold. Work outside
//! the group is not held back. Both backends, both claim paths.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewThread, NewUsageLedgerEntry, NewWorkspace, PriceSnapshot,
    Thread, TokenUsage,
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
                name: "fanout".into(),
            })
            .await
            .expect("ws");
        let mut members = Vec::new();
        for handle in ["carol", "alice", "bob", "dave"] {
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
        let (carol, alice, bob, dave) = (&members[0], &members[1], &members[2], &members[3]);
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "fanout".into(),
                topic: None,
                private: false,
            })
            .await
            .expect("channel");
        let new_thread = |title: &'static str, parent| NewThread {
            channel_id: channel.id,
            parent_thread_id: parent,
            title: Some(title.into()),
            description: None,
            created_by: None,
        };
        let parent = store
            .create_thread(new_thread("recipe", None))
            .await
            .expect("parent");
        let mut children = Vec::new();
        for title in ["child 1", "child 2", "child 3"] {
            children.push(
                store
                    .create_thread(new_thread(title, Some(parent.id)))
                    .await
                    .expect("child"),
            );
        }
        let loner = store
            .create_thread(new_thread("unrelated", None))
            .await
            .expect("loner");

        // Oldest first: the parent, then the first child.
        let p = claim(store, channel.id, carol.id, with_event)
            .await
            .expect("parent claim");
        assert_eq!(p.id, parent.id);
        let first = claim(store, channel.id, alice.id, with_event)
            .await
            .expect("first child");
        assert_eq!(first.id, children[0].id);

        // The other children wait for the first to warm the prefix; the
        // unrelated thread is still handed out, then nothing is left.
        let b = claim(store, channel.id, bob.id, with_event)
            .await
            .expect("loner claim");
        assert_eq!(b.id, loner.id, "siblings wait; other work does not");
        assert!(
            claim(store, channel.id, dave.id, with_event)
                .await
                .is_none(),
            "no sibling fans out before the first reports usage"
        );

        // The first child's first usage report: the prefix is warm.
        store
            .report_accounted_usage(&NewUsageLedgerEntry {
                usage_report_id: uuid::Uuid::new_v4(),
                thread_id: first.id,
                reporter: alice.id,
                claim_lease_id: first.claim_lease_id.expect("leased"),
                model: "provider/model".into(),
                tokens: TokenUsage {
                    input: 1_000_000,
                    ..Default::default()
                },
                usd_micros: 10,
                price_snapshot: PriceSnapshot {
                    input_usd_micros_per_million: 10,
                    ..Default::default()
                },
                turns: 1,
                evidence: Default::default(),
            })
            .await
            .expect("usage");

        let d = claim(store, channel.id, dave.id, with_event)
            .await
            .expect("fan out");
        assert_eq!(d.id, children[1].id);
        let b = claim(store, channel.id, bob.id, with_event)
            .await
            .expect("fan out again");
        assert_eq!(b.id, children[2].id);
    }
}

#[tokio::test]
async fn siblings_warm_then_fan_out_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn siblings_warm_then_fan_out_postgres() {
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
