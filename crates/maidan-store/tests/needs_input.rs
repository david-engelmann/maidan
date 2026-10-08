//! The threads whose agent asked a question (`needs_input`), on both backends:
//! one workspace's, open ones only, oldest question first, with the thread's
//! title and owner.

use std::time::Duration;

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn thread_with_status(
    store: &dyn Store,
    channel: ChannelId,
    agent: MemberId,
    title: &str,
    status: DeclaredStatus,
) -> ThreadId {
    let thread = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
        })
        .await
        .expect("thread");
    store.claim_thread(thread.id, agent).await.expect("claim");
    store
        .declare_thread_status(thread.id, status, format!("{title}?"), agent)
        .await
        .expect("declare");
    thread.id
}

async fn run_suite(store: &dyn Store) {
    let mut channels = Vec::new();
    let mut agents = Vec::new();
    for name in ["asks", "elsewhere"] {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .expect("workspace");
        let agent = store
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
                name: "work".into(),
                topic: None,
                private: false,
            })
            .await
            .expect("channel");
        channels.push((ws.id, channel.id));
        agents.push(agent.id);
    }
    let ((ws, channel), agent) = (channels[0], agents[0]);

    let first =
        thread_with_status(store, channel, agent, "first", DeclaredStatus::NeedsInput).await;
    store
        .set_thread_owner(first, Some(agent))
        .await
        .expect("owner");
    thread_with_status(store, channel, agent, "working", DeclaredStatus::Working).await;
    let closed =
        thread_with_status(store, channel, agent, "closed", DeclaredStatus::NeedsInput).await;
    for action in [ThreadAction::StartReview, ThreadAction::Close] {
        store
            .transition_thread(closed, agent, action)
            .await
            .expect("close");
    }
    tokio::time::sleep(Duration::from_millis(5)).await;
    let second =
        thread_with_status(store, channel, agent, "second", DeclaredStatus::NeedsInput).await;
    let (other_channel, other_agent) = (channels[1].1, agents[1]);
    thread_with_status(
        store,
        other_channel,
        other_agent,
        "tenant b",
        DeclaredStatus::NeedsInput,
    )
    .await;

    let asked = store.list_threads_needing_input(ws).await.expect("list");
    let got: Vec<_> = asked
        .iter()
        .map(|(tid, title, owner, q)| (*tid, title.as_deref(), *owner, q.note.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (first, Some("first"), Some(agent), "first?"),
            (second, Some("second"), None, "second?"),
        ],
        "this workspace's open questions, oldest first; not another status, \
         not a closed thread, not another workspace's"
    );

    store.clear_thread_status(first).await.expect("clear");
    let left = store.list_threads_needing_input(ws).await.expect("list");
    assert_eq!(left.len(), 1, "an answered question is gone");
    assert_eq!(left[0].0, second);
}

#[tokio::test]
async fn questions_list_open_threads_of_one_workspace_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn questions_list_open_threads_of_one_workspace_on_postgres() {
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
