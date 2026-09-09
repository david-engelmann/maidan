//! Per-workspace WIP limit + a member's live-claim count (Cluster 362, G11).
//! Both backends.

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
        .create_workspace(NewWorkspace { name: "wip".into() })
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
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");

    // The limit is unset (unlimited) until configured, then upserts, then clears.
    assert_eq!(store.get_wip_limit(ws.id).await.expect("get"), None);
    store.set_wip_limit(ws.id, Some(2)).await.expect("set");
    assert_eq!(store.get_wip_limit(ws.id).await.expect("get"), Some(2));
    store.set_wip_limit(ws.id, Some(0)).await.expect("freeze");
    assert_eq!(store.get_wip_limit(ws.id).await.expect("get"), Some(0));
    store.set_wip_limit(ws.id, None).await.expect("clear");
    assert_eq!(store.get_wip_limit(ws.id).await.expect("get"), None);

    // Live-claim count reflects durable assignments, excludes terminal + unassigned.
    let mk_thread = |title: &str| {
        let channel_id = channel.id;
        let title = title.to_string();
        async move {
            store
                .create_thread(NewThread {
                    channel_id,
                    parent_thread_id: None,
                    title: Some(title),
                })
                .await
                .expect("thread")
        }
    };
    let t1 = mk_thread("t1").await;
    let t2 = mk_thread("t2").await;

    assert_eq!(store.count_live_claims(member.id).await.expect("count"), 0);
    store.assign_thread(t1.id, member.id).await.expect("assign");
    assert_eq!(store.count_live_claims(member.id).await.expect("count"), 1);
    store.assign_thread(t2.id, member.id).await.expect("assign");
    assert_eq!(store.count_live_claims(member.id).await.expect("count"), 2);

    // Unassigning drops the count.
    store.unassign_thread(t2.id).await.expect("unassign");
    assert_eq!(store.count_live_claims(member.id).await.expect("count"), 1);

    // A terminal thread is not live work, even while still assigned.
    store
        .transition_thread(t1.id, member.id, maidan_fsm::ThreadAction::StartReview)
        .await
        .expect("start");
    store
        .transition_thread(t1.id, member.id, maidan_fsm::ThreadAction::Close)
        .await
        .expect("close");
    assert_eq!(
        store.count_live_claims(member.id).await.expect("count"),
        0,
        "a closed thread does not count toward WIP"
    );
}

#[tokio::test]
async fn wip_limit_and_live_claim_count_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn wip_limit_and_live_claim_count_postgres() {
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
