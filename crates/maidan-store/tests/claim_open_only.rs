//! `claim_next` hands out only `open` threads. A thread under review, closed or
//! archived is finished work: releasing it, or letting its lease lapse, must not
//! put it back in the queue, or a waiter that hands its result to review and
//! lets go picks the same task up again. Both backends, both claim paths.

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, Thread};
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

async fn claim(
    store: &dyn Store,
    channel: maidan_types::ChannelId,
    member: maidan_types::MemberId,
    with_event: bool,
) -> Option<Thread> {
    if with_event {
        store
            .claim_next_thread_with_event(channel, member, Some(60))
            .await
            .expect("claim")
            .0
    } else {
        store
            .claim_next_thread(channel, member, Some(60))
            .await
            .expect("claim")
    }
}

async fn run_suite(store: &dyn Store) {
    for with_event in [false, true] {
        let ws = store
            .create_workspace(NewWorkspace { name: "q".into() })
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
        let mut threads = Vec::new();
        for title in ["reviewing", "closed", "archived", "open"] {
            threads.push(
                store
                    .create_thread(NewThread {
                        channel_id: channel.id,
                        parent_thread_id: None,
                        title: Some(title.into()),
                    })
                    .await
                    .expect("thread"),
            );
        }
        let actions: [&[ThreadAction]; 3] = [
            &[ThreadAction::StartReview],
            &[ThreadAction::StartReview, ThreadAction::Close],
            &[
                ThreadAction::StartReview,
                ThreadAction::Close,
                ThreadAction::Archive,
            ],
        ];
        for (thread, path) in threads.iter().zip(actions) {
            for action in path {
                store
                    .transition_thread(thread.id, worker.id, *action)
                    .await
                    .expect("transition");
            }
        }

        // Older finished threads outrank the open one by age; only it is handed out.
        let got = claim(store, channel.id, worker.id, with_event)
            .await
            .expect("the open thread");
        assert_eq!(got.id, threads[3].id);
        assert!(claim(store, channel.id, worker.id, with_event)
            .await
            .is_none());

        // The waiter's own ending: result, hand to review, let go. Not reclaimed.
        store
            .set_thread_result(got.id, worker.id, &serde_json::json!({"ok": true}))
            .await
            .expect("result");
        store
            .transition_thread(got.id, worker.id, ThreadAction::StartReview)
            .await
            .expect("start review");
        let lease = got.claim_lease_id.expect("leased");
        store
            .release_claim(got.id, worker.id, lease)
            .await
            .expect("release");
        assert!(
            claim(store, channel.id, worker.id, with_event)
                .await
                .is_none(),
            "a thread handed to review came back to the queue"
        );
    }
}

#[tokio::test]
async fn claim_next_hands_out_only_open_threads_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn claim_next_hands_out_only_open_threads_postgres() {
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
