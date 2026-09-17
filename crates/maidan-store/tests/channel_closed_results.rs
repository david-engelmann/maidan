//! In-channel closed results: `claim_next`'s pack lists terminal-thread results
//! so the next claimer sees decisions already made in the channel. Both
//! backends. No routes yet — store foundation.

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId, ThreadState,
};
use serde_json::json;
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

async fn close_thread(store: &dyn Store, id: ThreadId, actor: maidan_types::MemberId) {
    store
        .transition_thread(id, actor, ThreadAction::StartReview)
        .await
        .expect("review");
    store
        .transition_thread(id, actor, ThreadAction::Close)
        .await
        .expect("close");
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "closed-results".into(),
        })
        .await
        .expect("ws");
    let member = store
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
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let other = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "elsewhere".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("other ch");

    let closed = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("ship postgres".into()),
        })
        .await
        .expect("closed");
    store
        .set_thread_result(closed.id, member.id, &json!({"decision": "postgres"}))
        .await
        .expect("result on closed");
    close_thread(store, closed.id, member.id).await;

    // Open thread with a result — still in flight, not a closed decision.
    let open = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("still working".into()),
        })
        .await
        .expect("open");
    store
        .set_thread_result(open.id, member.id, &json!({"wip": true}))
        .await
        .expect("result on open");

    // Closed, but no result — a land without a decision payload.
    let closed_empty = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("closed with nothing".into()),
        })
        .await
        .expect("closed empty");
    close_thread(store, closed_empty.id, member.id).await;

    // Closed with a result, but in a different channel.
    let elsewhere = store
        .create_thread(NewThread {
            channel_id: other.id,
            parent_thread_id: None,
            title: Some("other channel".into()),
        })
        .await
        .expect("elsewhere");
    store
        .set_thread_result(elsewhere.id, member.id, &json!({"nope": true}))
        .await
        .expect("elsewhere result");
    close_thread(store, elsewhere.id, member.id).await;

    // Archived (closed → archived) with a result also counts as terminal.
    let archived = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("old call".into()),
        })
        .await
        .expect("archived");
    store
        .set_thread_result(archived.id, member.id, &json!({"decision": "archive-me"}))
        .await
        .expect("archived result");
    close_thread(store, archived.id, member.id).await;
    store
        .transition_thread(archived.id, member.id, ThreadAction::Archive)
        .await
        .expect("archive");

    let listed = store
        .list_channel_closed_results(channel.id, None, 50)
        .await
        .expect("list");
    let ids: Vec<_> = listed.iter().map(|r| r.thread_id).collect();
    assert!(ids.contains(&closed.id), "closed-with-result is listed");
    assert!(ids.contains(&archived.id), "archived-with-result is listed");
    assert!(
        !ids.contains(&open.id),
        "open-with-result is not a closed decision"
    );
    assert!(
        !ids.contains(&closed_empty.id),
        "closed-without-result has no decision payload"
    );
    assert!(
        !ids.contains(&elsewhere.id),
        "a closed result in another channel is out of scope"
    );
    assert_eq!(listed.len(), 2, "exactly the two terminal-with-result rows");
    assert_eq!(listed[0].state, ThreadState::Archived);
    assert_eq!(listed[1].state, ThreadState::Closed);
    assert_eq!(listed[1].result, json!({"decision": "postgres"}));
    assert_eq!(listed[1].title.as_deref(), Some("ship postgres"));

    // Newest first: archived was written after closed.
    assert_eq!(listed[0].thread_id, archived.id);
    assert!(listed[0].produced_at >= listed[1].produced_at);

    // Exclude the claimer's own thread (even though it isn't closed — the
    // claimer of `open` shouldn't see a self-row if they later close it).
    let without_closed = store
        .list_channel_closed_results(channel.id, Some(closed.id), 50)
        .await
        .expect("exclude");
    assert_eq!(without_closed.len(), 1);
    assert_eq!(without_closed[0].thread_id, archived.id);

    // Limit is honored (and clamped — 0 still returns at least the newest).
    let one = store
        .list_channel_closed_results(channel.id, None, 1)
        .await
        .expect("limit 1");
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].thread_id, archived.id);

    let empty_ch = store
        .list_channel_closed_results(other.id, Some(elsewhere.id), 10)
        .await
        .expect("other channel excluding its only closed result");
    assert!(empty_ch.is_empty());
}

#[tokio::test]
async fn channel_closed_results_lists_terminal_in_channel_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn channel_closed_results_lists_terminal_in_channel_postgres() {
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
