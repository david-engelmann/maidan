//! Slack projector channel links (Cluster 308): link (upsert) / get / list /
//! unlink a Slack channel → Maidan channel/thread/member mapping. Both backends.

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewSlackChannelLink, NewThread, NewWorkspace,
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
        .create_workspace(NewWorkspace {
            name: "slack".into(),
        })
        .await
        .expect("ws");
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "slackbot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("slack".into()),
        })
        .await
        .expect("thread");

    // Unlinked to start.
    assert!(store
        .get_slack_channel_link("C123")
        .await
        .expect("get none")
        .is_none());

    // Link, then resolve.
    let link = store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: "C123".into(),
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: bot.id,
        })
        .await
        .expect("link");
    assert_eq!(link.slack_channel_id, "C123");
    assert_eq!(link.thread_id, thread.id);
    let got = store
        .get_slack_channel_link("C123")
        .await
        .expect("get")
        .expect("some");
    assert_eq!(got.member_id, bot.id);
    assert_eq!(
        store
            .list_slack_channel_links(ws.id)
            .await
            .expect("list")
            .len(),
        1
    );

    // Re-linking the same Slack channel upserts (still one row).
    let thread2 = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("slack2".into()),
        })
        .await
        .expect("thread2");
    store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: "C123".into(),
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread2.id,
            member_id: bot.id,
        })
        .await
        .expect("relink");
    assert_eq!(
        store
            .get_slack_channel_link("C123")
            .await
            .expect("get2")
            .expect("some2")
            .thread_id,
        thread2.id,
        "re-link points at the new thread"
    );
    assert_eq!(
        store
            .list_slack_channel_links(ws.id)
            .await
            .expect("list2")
            .len(),
        1
    );

    // Unlink.
    assert!(store.unlink_slack_channel("C123").await.expect("unlink"));
    assert!(
        !store
            .unlink_slack_channel("C123")
            .await
            .expect("unlink again"),
        "second unlink removes nothing"
    );
    assert!(store
        .get_slack_channel_link("C123")
        .await
        .expect("get after unlink")
        .is_none());
}

#[tokio::test]
async fn slack_channel_link_crud_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn slack_channel_link_crud_postgres() {
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
