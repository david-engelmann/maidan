//! A Slack channel link and a GitHub issue link belong to the workspace that
//! made them. Two workspaces link the same channel and the same issue, each
//! for itself, and neither learns of the other. An inbound delivery finds the
//! links its route serves: a workspace's own route, its own links, and the
//! server's route, the links of workspaces the operator opened that stored no
//! connection of their own. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{
    ChannelId, ConnectionSurface, InboundScope, MemberId, MemberKind, NewChannel,
    NewGithubIssueLink, NewMember, NewSecret, NewSlackChannelLink, NewThread, NewWorkspace,
    ThreadId, WorkspaceId,
};

async fn sqlite() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
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

struct Room {
    ws: WorkspaceId,
    member: MemberId,
    channel: ChannelId,
    thread: ThreadId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
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
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    Room {
        ws: ws.id,
        member: member.id,
        channel: channel.id,
        thread: thread.id,
    }
}

async fn link_slack(store: &dyn Store, r: &Room) {
    store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: "C1".into(),
            workspace_id: r.ws,
            channel_id: r.channel,
            thread_id: r.thread,
            member_id: r.member,
        })
        .await
        .expect("link slack");
}

async fn link_github(store: &dyn Store, r: &Room) {
    store
        .link_github_issue(NewGithubIssueLink {
            repo: "o/r".into(),
            issue_number: 7,
            workspace_id: r.ws,
            channel_id: r.channel,
            thread_id: r.thread,
            member_id: r.member,
        })
        .await
        .expect("link github");
}

fn threads(links: &[maidan_types::SlackChannelLink]) -> Vec<ThreadId> {
    links.iter().map(|l| l.thread_id).collect()
}

async fn run_suite(store: &dyn Store) {
    let a = room(store, "a").await;
    let b = room(store, "b").await;

    // Each links the same channel and the same issue for itself.
    link_slack(store, &a).await;
    link_slack(store, &b).await;
    link_github(store, &a).await;
    link_github(store, &b).await;
    assert_eq!(
        store
            .get_slack_channel_link(a.ws, "C1")
            .await
            .unwrap()
            .unwrap()
            .thread_id,
        a.thread
    );
    assert_eq!(
        store
            .get_slack_channel_link(b.ws, "C1")
            .await
            .unwrap()
            .unwrap()
            .thread_id,
        b.thread
    );
    assert_eq!(
        store
            .get_github_issue_link(a.ws, "o/r", 7)
            .await
            .unwrap()
            .unwrap()
            .thread_id,
        a.thread
    );
    assert_eq!(store.list_slack_channel_links(a.ws).await.unwrap().len(), 1);

    // A workspace's own route finds its own link, and the server's route finds
    // the links of the workspaces it serves (both, while neither has stored a
    // connection of its own).
    let own_a = InboundScope::Workspace(a.ws);
    let served = InboundScope::Instance(ConnectionSurface::Slack);
    let served_gh = InboundScope::Instance(ConnectionSurface::Github);
    assert_eq!(
        threads(
            &store
                .list_slack_channel_links_for_channel(own_a, "C1")
                .await
                .unwrap()
        ),
        vec![a.thread]
    );
    assert_eq!(
        store
            .list_slack_channel_links_for_channel(served, "C1")
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store
            .list_github_issue_links_for_issue(served_gh, "o/r", 7)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(store
        .list_slack_channel_links_for_channel(own_a, "C2")
        .await
        .unwrap()
        .is_empty());

    // B connects its own Slack: the server's route no longer reaches it for
    // Slack, and still reaches it for GitHub, which it did not connect.
    store
        .create_secret(NewSecret {
            workspace_id: b.ws,
            name: ConnectionSurface::Slack.secret_name().into(),
            value_ciphertext: "not a real ciphertext".into(),
            created_by: b.member,
        })
        .await
        .unwrap();
    assert_eq!(
        threads(
            &store
                .list_slack_channel_links_for_channel(served, "C1")
                .await
                .unwrap()
        ),
        vec![a.thread]
    );
    assert_eq!(
        store
            .list_github_issue_links_for_issue(served_gh, "o/r", 7)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        threads(
            &store
                .list_slack_channel_links_for_channel(InboundScope::Workspace(b.ws), "C1")
                .await
                .unwrap()
        ),
        vec![b.thread]
    );

    // One workspace's disable and unlink leave the other's link alone.
    assert!(store.disable_slack_channel_link(a.ws, "C1").await.unwrap());
    assert!(store
        .get_slack_channel_link(b.ws, "C1")
        .await
        .unwrap()
        .unwrap()
        .disabled_at
        .is_none());
    assert!(store
        .disable_github_issue_link(a.ws, "o/r", 7)
        .await
        .unwrap());
    assert!(store
        .get_github_issue_link(b.ws, "o/r", 7)
        .await
        .unwrap()
        .unwrap()
        .disabled_at
        .is_none());
    assert!(store.unlink_slack_channel(a.ws, "C1").await.unwrap());
    assert!(store
        .get_slack_channel_link(a.ws, "C1")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .get_slack_channel_link(b.ws, "C1")
        .await
        .unwrap()
        .is_some());
    assert!(
        !store.unlink_github_issue(a.ws, "o/other", 7).await.unwrap(),
        "an unlink of a link the workspace does not hold removes nothing"
    );

    // A thread still takes one GitHub link: a second issue for the same
    // thread is refused whatever the workspace.
    let second = store
        .link_github_issue(NewGithubIssueLink {
            repo: "o/r".into(),
            issue_number: 8,
            workspace_id: b.ws,
            channel_id: b.channel,
            thread_id: b.thread,
            member_id: b.member,
        })
        .await;
    assert!(matches!(second, Err(StoreError::Conflict(_))), "{second:?}");

    // Erasing one workspace takes its links and leaves the other's.
    link_slack(store, &a).await;
    store.erase_workspace(a.ws).await.expect("erase a");
    assert!(store
        .get_slack_channel_link(b.ws, "C1")
        .await
        .unwrap()
        .is_some());
    assert!(store
        .get_github_issue_link(b.ws, "o/r", 7)
        .await
        .unwrap()
        .is_some());
    assert!(store
        .list_slack_channel_links_for_channel(InboundScope::Workspace(a.ws), "C1")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn two_workspaces_link_the_same_channel_and_issue_and_an_inbound_route_finds_only_what_it_serves_sqlite(
) {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn two_workspaces_link_the_same_channel_and_issue_and_an_inbound_route_finds_only_what_it_serves_postgres(
) {
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
