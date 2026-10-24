//! `claim_next` never hands out an open thread that already has a stored
//! result (#1499): that is finished work nobody closed, and handing it out
//! again re-runs it. A reviewer sending it back (review -> open after the
//! result) is the explicit re-queue that makes it claimable again. Both
//! backends, channel and workspace claims, with and without an engine.

use maidan_store::test_support::SetResultClaiming;
use std::time::Duration;

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    TaskAttributes, TaskEngine, Thread, WorkspaceId,
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
    SqliteStore::for_tests(pool)
}

#[derive(Clone, Copy)]
enum Scope {
    Channel(ChannelId),
    Workspace(WorkspaceId),
}

async fn claim(
    store: &dyn Store,
    scope: Scope,
    member: MemberId,
    lease_secs: i64,
    engine: Option<TaskEngine>,
) -> Option<Thread> {
    match scope {
        Scope::Channel(channel) => {
            store
                .claim_next_thread_with_event(channel, member, Some(lease_secs), None, engine, None)
                .await
                .expect("claim")
                .0
        }
        Scope::Workspace(ws) => {
            store
                .claim_next_workspace_thread_with_event(
                    ws,
                    member,
                    Some(lease_secs),
                    None,
                    engine,
                    None,
                )
                .await
                .expect("claim")
                .0
        }
    }
}

async fn task(
    store: &dyn Store,
    channel: ChannelId,
    title: &str,
    engine: Option<TaskEngine>,
) -> Thread {
    let thread = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    if engine.is_some() {
        store
            .set_thread_attributes(
                thread.id,
                TaskAttributes {
                    engine,
                    ..TaskAttributes::default()
                },
            )
            .await
            .expect("attributes");
    }
    thread
}

async fn run_suite(store: &dyn Store) {
    for (n, (workspace_scope, engine)) in [
        (false, None),
        (true, None),
        (false, Some(TaskEngine::Pi)),
        (true, Some(TaskEngine::Pi)),
    ]
    .into_iter()
    .enumerate()
    {
        let ws = store
            .create_workspace(NewWorkspace {
                name: format!("q{n}"),
            })
            .await
            .expect("ws");
        let worker = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "worker".into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .expect("member");
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "q".into(),
                topic: None,
                private: false,
            })
            .await
            .expect("channel");
        let scope = if workspace_scope {
            Scope::Workspace(ws.id)
        } else {
            Scope::Channel(channel.id)
        };
        let result = serde_json::json!({"ok": true});

        // Finished under a claim that then lapsed: not handed out again.
        let done = task(store, channel.id, "done", engine).await;
        let got = claim(store, scope, worker.id, 1, engine)
            .await
            .expect("the open thread");
        assert_eq!(got.id, done.id);
        store
            .set_thread_result_claiming(done.id, worker.id, &result)
            .await
            .expect("result");
        tokio::time::sleep(Duration::from_millis(1_200)).await;

        // Finished with no claim at all: not handed out either.
        let unclaimed = task(store, channel.id, "unclaimed", engine).await;
        store
            .set_thread_result_claiming(unclaimed.id, worker.id, &result)
            .await
            .expect("result");
        assert!(
            claim(store, scope, worker.id, 60, engine).await.is_none(),
            "a thread with a stored result was handed out again"
        );

        // Open work without a result is still handed out past them.
        let fresh = task(store, channel.id, "fresh", engine).await;
        let got = claim(store, scope, worker.id, 60, engine)
            .await
            .expect("the thread without a result");
        assert_eq!(got.id, fresh.id);

        // A reviewer sending the work back re-queues it explicitly.
        store
            .transition_thread(unclaimed.id, worker.id, ThreadAction::StartReview)
            .await
            .expect("start review");
        store
            .transition_thread(unclaimed.id, worker.id, ThreadAction::RequestChanges)
            .await
            .expect("send back");
        let got = claim(store, scope, worker.id, 60, engine)
            .await
            .expect("the sent-back thread");
        assert_eq!(got.id, unclaimed.id);
        assert!(claim(store, scope, worker.id, 60, engine).await.is_none());
    }
}

#[tokio::test]
async fn a_thread_with_a_stored_result_is_not_handed_out_again_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn a_thread_with_a_stored_result_is_not_handed_out_again_postgres() {
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
