//! Thread dispatch-park store (Cluster 363, G3): mark/clear/get + channel list.
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
        .create_workspace(NewWorkspace { name: "u".into() })
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

    // Claimable by default.
    assert!(store
        .get_thread_unclaimable(t1.id)
        .await
        .expect("get")
        .is_none());
    assert!(store
        .list_unclaimable_threads(channel.id)
        .await
        .expect("list")
        .is_empty());

    // Park t1 with a reason.
    let marked = store
        .mark_thread_unclaimable(t1.id, "needs triage", member.id)
        .await
        .expect("mark");
    assert_eq!(marked.thread_id, t1.id);
    assert_eq!(marked.reason, "needs triage");
    assert_eq!(marked.marked_by, member.id);

    let got = store
        .get_thread_unclaimable(t1.id)
        .await
        .expect("get")
        .expect("parked");
    assert_eq!(got.reason, "needs triage");

    // Re-mark upserts the reason.
    store
        .mark_thread_unclaimable(t1.id, "waiting on external", member.id)
        .await
        .expect("remark");
    assert_eq!(
        store
            .get_thread_unclaimable(t1.id)
            .await
            .expect("get")
            .expect("parked")
            .reason,
        "waiting on external"
    );

    // Park t2 too; the channel list shows both.
    store
        .mark_thread_unclaimable(t2.id, "broken", member.id)
        .await
        .expect("mark2");
    let listed = store
        .list_unclaimable_threads(channel.id)
        .await
        .expect("list");
    assert_eq!(listed.len(), 2);

    // Clearing returns true once, then false (idempotent no-op).
    assert!(store.mark_thread_claimable(t1.id).await.expect("clear"));
    assert!(!store
        .mark_thread_claimable(t1.id)
        .await
        .expect("clear again"));
    assert!(store
        .get_thread_unclaimable(t1.id)
        .await
        .expect("get")
        .is_none());
    assert_eq!(
        store
            .list_unclaimable_threads(channel.id)
            .await
            .expect("list")
            .len(),
        1,
        "only t2 remains parked"
    );
}

#[tokio::test]
async fn thread_unclaimable_mark_clear_get_list_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn thread_unclaimable_mark_clear_get_list_postgres() {
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
