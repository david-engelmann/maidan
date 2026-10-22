//! `find_thread_by_external_ref`: a Slack message or channel, or a pull
//! request, finds its thread in the caller's workspace and never another's,
//! on both backends.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Tenant {
    ws: WorkspaceId,
    channel: ChannelId,
    member: MemberId,
}

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Tenant {
        ws: ws.id,
        channel: channel.id,
        member: member.id,
    }
}

async fn thread(store: &dyn Store, t: &Tenant, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: t.channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id
}

/// A delivered result on `thread`, posted to `target` as `posted_ts`.
async fn delivered(store: &dyn Store, thread: ThreadId, target: EgressTarget, posted: &str) {
    let row = store
        .arm_result_delivery(thread, &target, Utc::now())
        .await
        .unwrap()
        .unwrap();
    store
        .mark_result_delivered(row.id, Some(posted), Utc::now())
        .await
        .unwrap();
}

fn pull(repo: &str, number: i64) -> ExternalThreadRef {
    ExternalThreadRef::Pull {
        repo: repo.into(),
        number,
    }
}

fn slack(channel: &str, ts: Option<&str>) -> ExternalThreadRef {
    ExternalThreadRef::Slack {
        channel: channel.into(),
        ts: ts.map(Into::into),
    }
}

async fn run_suite(store: &dyn Store) {
    let a = tenant(store, "a").await;
    let b = tenant(store, "b").await;
    let find = |ws: WorkspaceId, reference: ExternalThreadRef| async move {
        store
            .find_thread_by_external_ref(ws, &reference)
            .await
            .unwrap()
    };

    // A tracked pull request finds its thread, ahead of a link to the same
    // number, and only in its own workspace.
    let tracked = thread(store, &a, "tracked").await;
    store
        .register_change_pull(a.ws, tracked, "example/app", 7, None)
        .await
        .unwrap();
    let linked = thread(store, &a, "linked").await;
    store
        .link_github_issue(NewGithubIssueLink {
            repo: "Example/App".into(),
            issue_number: 7,
            workspace_id: a.ws,
            channel_id: a.channel,
            thread_id: linked,
            member_id: a.member,
        })
        .await
        .unwrap();
    assert_eq!(find(a.ws, pull("example/app", 7)).await, Some(tracked));
    assert_eq!(find(b.ws, pull("example/app", 7)).await, None, "tenant B");

    // A pull request only commented on by a result delivery finds that thread.
    let commenter = thread(store, &a, "commenter").await;
    delivered(
        store,
        commenter,
        EgressTarget::Github {
            repo: "Example/Lib".into(),
            issue_number: 12,
        },
        "991",
    )
    .await;
    assert_eq!(find(a.ws, pull("example/lib", 12)).await, Some(commenter));
    assert_eq!(find(a.ws, pull("example/lib", 13)).await, None);

    // A Slack thread a result replied in, or the message it was posted as.
    let replier = thread(store, &a, "replier").await;
    delivered(
        store,
        replier,
        EgressTarget::Slack {
            channel_id: "C0AAA".into(),
            thread_ts: Some("1700000000.000100".into()),
        },
        "1700000000.000200",
    )
    .await;
    let poster = thread(store, &a, "poster").await;
    delivered(
        store,
        poster,
        EgressTarget::Slack {
            channel_id: "C0BBB".into(),
            thread_ts: None,
        },
        "1700000001.000300",
    )
    .await;
    assert_eq!(
        find(a.ws, slack("C0AAA", Some("1700000000.000100"))).await,
        Some(replier),
        "the Slack thread asked in"
    );
    assert_eq!(
        find(a.ws, slack("C0AAA", Some("1700000000.000200"))).await,
        Some(replier),
        "the reply it posted"
    );
    assert_eq!(
        find(a.ws, slack("C0BBB", Some("1700000001.000300"))).await,
        Some(poster)
    );
    assert_eq!(
        find(a.ws, slack("C0BBB", Some("1700000001.000301"))).await,
        None
    );
    assert_eq!(
        find(b.ws, slack("C0AAA", Some("1700000000.000100"))).await,
        None,
        "tenant B"
    );

    // A channel alone finds the thread it is linked to.
    let bridged = thread(store, &a, "bridged").await;
    store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: "C0CCC".into(),
            workspace_id: a.ws,
            channel_id: a.channel,
            thread_id: bridged,
            member_id: a.member,
        })
        .await
        .unwrap();
    assert_eq!(find(a.ws, slack("C0CCC", None)).await, Some(bridged));
    assert_eq!(find(b.ws, slack("C0CCC", None)).await, None, "tenant B");
}

#[tokio::test]
async fn a_thread_is_found_by_its_slack_or_pull_request_reference_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn a_thread_is_found_by_its_slack_or_pull_request_reference_postgres() {
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
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::for_tests(pool)).await;
}
