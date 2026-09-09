//! Buried decisions (Cluster 359, N2): task results produced by *someone else*
//! in a channel/thread the member follows, since a watermark. Both backends.

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
        .create_workspace(NewWorkspace { name: "d".into() })
        .await
        .expect("ws");
    let mk = |handle: &str| NewMember {
        workspace_id: ws.id,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Agent,
    };
    let follower = store.create_member(mk("follower")).await.expect("follower");
    let producer = store.create_member(mk("producer")).await.expect("producer");

    let followed = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "followed".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("followed ch");
    let other = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "other".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("other ch");
    store
        .follow_channel(follower.id, followed.id)
        .await
        .expect("follow");

    let epoch = chrono::DateTime::from_timestamp(0, 0).unwrap();

    // A decision by the producer in the followed channel → buried for the follower.
    let t1 = store
        .create_thread(NewThread {
            channel_id: followed.id,
            parent_thread_id: None,
            title: Some("ship it".into()),
        })
        .await
        .expect("t1");
    store
        .set_thread_result(t1.id, producer.id, &serde_json::json!("approved v2"))
        .await
        .expect("result 1");

    // A decision by the FOLLOWER themselves → not "buried" (they made it).
    let t2 = store
        .create_thread(NewThread {
            channel_id: followed.id,
            parent_thread_id: None,
            title: Some("my own call".into()),
        })
        .await
        .expect("t2");
    store
        .set_thread_result(t2.id, follower.id, &serde_json::json!("done"))
        .await
        .expect("result 2");

    // A decision in an UNFOLLOWED channel → not surfaced.
    let t3 = store
        .create_thread(NewThread {
            channel_id: other.id,
            parent_thread_id: None,
            title: Some("elsewhere".into()),
        })
        .await
        .expect("t3");
    store
        .set_thread_result(t3.id, producer.id, &serde_json::json!("nope"))
        .await
        .expect("result 3");

    let buried = store
        .buried_decisions_for_member(follower.id, epoch, 50)
        .await
        .expect("buried");
    assert_eq!(
        buried.len(),
        1,
        "only the followed-channel other-authored one"
    );
    assert_eq!(buried[0].thread_id, t1.id);
    assert_eq!(buried[0].channel_id, followed.id);
    assert_eq!(buried[0].thread_title.as_deref(), Some("ship it"));
    assert_eq!(buried[0].produced_by, producer.id);
    assert_eq!(buried[0].result, serde_json::json!("approved v2"));

    // The `since` watermark excludes older decisions.
    let future = chrono::Utc::now() + chrono::Duration::hours(1);
    assert!(store
        .buried_decisions_for_member(follower.id, future, 50)
        .await
        .expect("since future")
        .is_empty());

    // A member following the THREAD (not the channel) also sees its decision.
    let onlooker = store.create_member(mk("onlooker")).await.expect("onlooker");
    store
        .follow_thread(onlooker.id, t3.id)
        .await
        .expect("follow thread");
    let for_onlooker = store
        .buried_decisions_for_member(onlooker.id, epoch, 50)
        .await
        .expect("onlooker buried");
    assert_eq!(for_onlooker.len(), 1);
    assert_eq!(for_onlooker[0].thread_id, t3.id);
}

#[tokio::test]
async fn buried_decisions_scoped_to_follows_and_others_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn buried_decisions_scoped_to_follows_and_others_postgres() {
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
