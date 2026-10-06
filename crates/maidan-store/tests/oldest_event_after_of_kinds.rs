//! `Store::oldest_event_after_of_kinds`: the oldest event of the wanted kinds
//! after a log position, so readiness can tell an indexer that is behind from
//! one with nothing to index. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Event, EventKind, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
    SEARCH_PROJECTOR_KINDS,
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

async fn run_suite(store: &dyn Store) {
    assert_eq!(
        store
            .oldest_event_after_of_kinds(0, SEARCH_PROJECTOR_KINDS)
            .await
            .expect("empty log"),
        None
    );

    let ws = store
        .create_workspace(NewWorkspace {
            name: "oldest-after".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let (th, _) = store
        .create_thread_with_event(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
            description: None,
        })
        .await
        .expect("thread");
    let post = |body: &str| NewMessage {
        thread_id: th.id,
        author_id: member.id,
        body: body.into(),
        metadata: serde_json::json!({}),
        content: None,
    };

    let joined = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .expect("joined");
    assert_eq!(
        store
            .oldest_event_after_of_kinds(0, SEARCH_PROJECTOR_KINDS)
            .await
            .expect("only non-message events"),
        None,
        "events of other kinds are not indexable work"
    );

    let (_, first) = store
        .post_message_with_event(post("one"), None)
        .await
        .expect("first post");
    let (_, second) = store
        .post_message_with_event(post("two"), None)
        .await
        .expect("second post");
    assert_eq!(first.kind, EventKind::MessagePosted);
    assert!(first.id > joined.id && second.id > first.id);

    let oldest = store
        .oldest_event_after_of_kinds(0, SEARCH_PROJECTOR_KINDS)
        .await
        .expect("oldest")
        .expect("a message is pending");
    assert_eq!(oldest.0, first.id, "the oldest pending message, by id");
    assert!(
        (chrono::Utc::now() - oldest.1).num_seconds().abs() < 3600,
        "inserted_at is the store's stamp for this row"
    );

    let next = store
        .oldest_event_after_of_kinds(first.id, SEARCH_PROJECTOR_KINDS)
        .await
        .expect("after first")
        .expect("second pending");
    assert_eq!(next.0, second.id);

    assert_eq!(
        store
            .oldest_event_after_of_kinds(second.id, SEARCH_PROJECTOR_KINDS)
            .await
            .expect("caught up"),
        None
    );
    assert_eq!(
        store
            .oldest_event_after_of_kinds(0, &[])
            .await
            .expect("no kinds"),
        None
    );
}

#[tokio::test]
async fn oldest_event_after_of_kinds_filters_by_kind_and_position_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn oldest_event_after_of_kinds_filters_by_kind_and_position_postgres() {
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
