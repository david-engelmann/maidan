//! GitHub projector issue/PR links (Cluster 311): link (upsert) / get / by-thread /
//! list / unlink a (repo, issue) → Maidan channel/thread/member mapping. Both backends.

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    MemberKind, NewChannel, NewGithubIssueLink, NewMember, NewThread, NewWorkspace,
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
        .create_workspace(NewWorkspace { name: "gh".into() })
        .await
        .expect("ws");
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "ghbot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "eng".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("issue-42".into()),
        })
        .await
        .expect("thread");

    assert!(store
        .get_github_issue_link("o/r", 42)
        .await
        .expect("get none")
        .is_none());

    let link = store
        .link_github_issue(NewGithubIssueLink {
            repo: "o/r".into(),
            issue_number: 42,
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: bot.id,
        })
        .await
        .expect("link");
    assert_eq!(link.repo, "o/r");
    assert_eq!(link.issue_number, 42);

    let got = store
        .get_github_issue_link("o/r", 42)
        .await
        .expect("get")
        .expect("some");
    assert_eq!(got.thread_id, thread.id);
    // Reverse lookup by thread (egress path).
    let by_thread = store
        .get_github_issue_link_by_thread(thread.id)
        .await
        .expect("by thread")
        .expect("some");
    assert_eq!(by_thread.issue_number, 42);
    assert_eq!(
        store
            .list_github_issue_links(ws.id)
            .await
            .expect("list")
            .len(),
        1
    );

    // A different issue number is a distinct link (composite key).
    let thread2 = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("issue-43".into()),
        })
        .await
        .expect("thread2");
    store
        .link_github_issue(NewGithubIssueLink {
            repo: "o/r".into(),
            issue_number: 43,
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread2.id,
            member_id: bot.id,
        })
        .await
        .expect("link2");
    assert_eq!(
        store
            .list_github_issue_links(ws.id)
            .await
            .expect("list2")
            .len(),
        2
    );

    // Unlink one.
    assert!(store.unlink_github_issue("o/r", 42).await.expect("unlink"));
    assert!(
        !store
            .unlink_github_issue("o/r", 42)
            .await
            .expect("unlink again"),
        "second unlink removes nothing"
    );
    assert!(store
        .get_github_issue_link("o/r", 42)
        .await
        .expect("get after unlink")
        .is_none());
    assert_eq!(
        store
            .list_github_issue_links(ws.id)
            .await
            .expect("list3")
            .len(),
        1
    );
}

#[tokio::test]
async fn github_issue_link_crud_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn github_issue_link_crud_postgres() {
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
