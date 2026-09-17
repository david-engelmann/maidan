//! GitHub projector issue/PR links: link (upsert) / get / by-thread / list /
//! unlink a (repo, issue) → Maidan channel/thread/member mapping. Both
//! backends. Also the cap: at most one GitHub link per claim, so one thread
//! cannot fan out to N GitHub issues.

use maidan_store::{prelude::*, run_sqlite_migrations};
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

    // Retry-then-disable: a broken link is turned off, and only the first
    // failure gets to announce it. Re-linking is the re-enable path.
    assert!(got.disabled_at.is_none());
    assert!(store
        .disable_github_issue_link("o/r", 42)
        .await
        .expect("disable"));
    let disabled_at = store
        .get_github_issue_link("o/r", 42)
        .await
        .expect("get3")
        .expect("some3")
        .disabled_at
        .expect("disabled");
    assert!(
        !store
            .disable_github_issue_link("o/r", 42)
            .await
            .expect("disable again"),
        "a second failure does not re-announce"
    );
    assert_eq!(
        store
            .get_github_issue_link("o/r", 42)
            .await
            .expect("get4")
            .expect("some4")
            .disabled_at,
        Some(disabled_at),
        "and does not reset when the link broke"
    );
    store
        .link_github_issue(NewGithubIssueLink {
            repo: "o/r".into(),
            issue_number: 42,
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: bot.id,
        })
        .await
        .expect("re-link");
    assert!(
        store
            .get_github_issue_link("o/r", 42)
            .await
            .expect("get5")
            .expect("some5")
            .disabled_at
            .is_none(),
        "re-linking re-enables egress"
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

    // --- at most one GitHub link per claim ---
    // thread2 holds o/r#43; thread's link was just unlinked.
    let link_to = |repo: &str, issue: i64, target: maidan_types::ThreadId| {
        let new = NewGithubIssueLink {
            repo: repo.into(),
            issue_number: issue,
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: target,
            member_id: bot.id,
        };
        async move { store.link_github_issue(new).await }
    };

    // A second, distinct issue on a thread that already has one is refused.
    let denied = link_to("o/r", 44, thread2.id).await;
    assert!(
        matches!(denied, Err(StoreError::Conflict(ref m)) if m.contains("GitHub link")),
        "a 2nd GitHub link on one claim must be refused, got {denied:?}"
    );

    // Re-linking the *same* issue to the same thread stays idempotent (the
    // (repo, issue_number) upsert must not trip the per-thread cap).
    link_to("o/r", 43, thread2.id)
        .await
        .expect("re-linking the same issue is idempotent");
    assert_eq!(
        store
            .list_github_issue_links(ws.id)
            .await
            .expect("list4")
            .len(),
        1
    );

    // An unlinked thread can take a link, and moving that link onto a thread
    // that already holds one is refused too (the upsert path, not just INSERT).
    link_to("o/r", 45, thread.id)
        .await
        .expect("a free thread takes a link");
    let moved_onto_taken = link_to("o/r", 45, thread2.id).await;
    assert!(
        matches!(moved_onto_taken, Err(StoreError::Conflict(ref m)) if m.contains("GitHub link")),
        "moving a link onto an already-linked claim must be refused, got {moved_onto_taken:?}"
    );

    // Moving it to a free thread works, and the old thread loses its link.
    let thread3 = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("issue-45".into()),
        })
        .await
        .expect("thread3");
    link_to("o/r", 45, thread3.id)
        .await
        .expect("moving a link to a free thread");
    assert_eq!(
        store
            .get_github_issue_link("o/r", 45)
            .await
            .expect("get 45")
            .expect("some")
            .thread_id,
        thread3.id
    );
    assert!(store
        .get_github_issue_link_by_thread(thread.id)
        .await
        .expect("by thread after move")
        .is_none());
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
