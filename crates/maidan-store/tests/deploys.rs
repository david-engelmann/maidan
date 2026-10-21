//! Deploys a deploy hook reports, on both backends: a deploy of a merged
//! pull request's commit is said once on its thread, a failure is said as a
//! failure, a commit nobody merged says nothing, and a deploy reaches only
//! the workspace whose pull request merged that commit. Postgres runs under
//! testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const REPO: &str = "example/app";

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    member: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "watcher".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        ws: ws.id,
        channel: channel.id,
        member,
    }
}

async fn task(store: &dyn Store, room: &Room) -> ThreadId {
    let thread = store
        .create_thread(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: Some(room.member),
        })
        .await
        .unwrap()
        .id;
    store
        .set_thread_owner(thread, Some(room.member))
        .await
        .unwrap();
    thread
}

/// Track `pull_number` on `thread` in `room` and record it merged at
/// `head`.
async fn merged(store: &dyn Store, room: &Room, thread: ThreadId, pull_number: i64, head: &str) {
    store
        .record_change_round(NewChangeRound {
            workspace_id: room.ws,
            thread_id: thread,
            repo: REPO.into(),
            pull_number,
            commit_sha: head.into(),
            base_sha: "b".repeat(40),
            base_behind: None,
            diff_lines: 1,
            paths: vec!["src/lib.rs".into()],
            base_branch: Some("main".into()),
            rebase: false,
            previous_head: None,
            linear_issue: None,
        })
        .await
        .unwrap();
    let at = chrono::Utc::now();
    store
        .close_change_pull(REPO, pull_number, Some(at), Some(head), at)
        .await
        .unwrap();
}

fn deploy(sha: &str, status: CiStatus) -> NewDeploy {
    NewDeploy {
        repo: "Example/App".into(),
        sha: sha.to_ascii_uppercase(),
        environment: "production".into(),
        status,
        url: Some("https://deploys.example/42".into()),
    }
}

async fn a_deploy_is_said_once_on_the_thread_that_merged_its_commit(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let r = room(s, "deploys").await;
    let thread = task(s, &r).await;
    let head = "a".repeat(40);
    merged(s, &r, thread, 7, &head).await;

    let posted = s
        .record_deploy(deploy(&head, CiStatus::Success))
        .await
        .unwrap();
    assert_eq!(posted.len(), 1);
    let notice = &posted[0].notice;
    assert_eq!(posted[0].thread_id, thread);
    assert_eq!(posted[0].author_id, Some(r.member));
    assert_eq!(notice.kind, ThreadNoticeKind::Deployed);
    assert_eq!(notice.environment.as_deref(), Some("production"));
    assert_eq!(notice.head_sha.as_deref(), Some(head.as_str()));
    assert_eq!(
        notice.sentence(),
        "example/app#7 is deployed to production at aaaaaaaaaaaa (https://deploys.example/42)."
    );
    assert!(
        s.record_deploy(deploy(&head, CiStatus::Success))
            .await
            .unwrap()
            .is_empty(),
        "a retried hook call says nothing again"
    );

    let failed = s
        .record_deploy(deploy(&head, CiStatus::Failure))
        .await
        .unwrap();
    assert_eq!(failed.len(), 1, "a failure is news of its own");
    assert!(failed[0]
        .notice
        .sentence()
        .starts_with("Deploy to production failed for example/app#7"));

    let read = s
        .list_thread_notices(thread, THREAD_NOTICE_LIMIT)
        .await
        .unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(
        read[1].deploy_url.as_deref(),
        Some("https://deploys.example/42")
    );
    assert!(
        s.record_deploy(deploy(&"c".repeat(40), CiStatus::Success))
            .await
            .unwrap()
            .is_empty(),
        "a commit no tracked pull request merged says nothing"
    );
}

async fn a_deploy_reaches_only_the_workspace_that_merged_its_commit(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let mine = room(s, "mine").await;
    let theirs = room(s, "theirs").await;
    let my_thread = task(s, &mine).await;
    let their_thread = task(s, &theirs).await;
    merged(s, &mine, my_thread, 7, &"a".repeat(40)).await;
    merged(s, &theirs, their_thread, 8, &"d".repeat(40)).await;

    let posted = s
        .record_deploy(deploy(&"a".repeat(40), CiStatus::Success))
        .await
        .unwrap();
    assert_eq!(
        posted.iter().map(|p| p.thread_id).collect::<Vec<_>>(),
        vec![my_thread]
    );
    assert!(
        s.list_thread_notices(their_thread, THREAD_NOTICE_LIMIT)
            .await
            .unwrap()
            .is_empty(),
        "the other workspace's thread heard nothing"
    );
}

async fn sqlite() -> Arc<dyn Store> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Arc::new(SqliteStore::for_tests(pool))
}

async fn postgres() -> Option<(testcontainers::ContainerAsync<Postgres>, Arc<dyn Store>)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some((container, Arc::new(PostgresStore::for_tests(pool))))
}

macro_rules! on_both_backends {
    ($($name:ident),* $(,)?) => {
        mod sqlite_backend {
            $(
                #[tokio::test]
                async fn $name() {
                    super::$name(super::sqlite().await).await;
                }
            )*
        }
        mod postgres_backend {
            $(
                #[tokio::test]
                async fn $name() {
                    let Some((_container, store)) = super::postgres().await else {
                        return;
                    };
                    super::$name(store).await;
                }
            )*
        }
    };
}

on_both_backends!(
    a_deploy_is_said_once_on_the_thread_that_merged_its_commit,
    a_deploy_reaches_only_the_workspace_that_merged_its_commit,
);
