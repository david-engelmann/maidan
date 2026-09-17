//! `Store::max_event_id` is the room head (`0` when empty). Both backends. This
//! is an event-log id, not a Postgres WAL LSN.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{Event, MemberKind, NewChannel, NewMember, NewWorkspace, RoomLsn};
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
    assert_eq!(
        store.max_event_id().await.expect("empty"),
        0,
        "empty log is RoomLsn::EMPTY"
    );
    assert_eq!(RoomLsn::from_max_id(0), RoomLsn::EMPTY);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "room-lsn-ws".into(),
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

    let e1 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .expect("e1");
    assert_eq!(store.max_event_id().await.expect("after 1"), e1.id);

    let e2 = store
        .append_event(&Event::ChannelCreated {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            channel: ch,
        })
        .await
        .expect("e2");
    let e3 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member,
        })
        .await
        .expect("e3");

    let max = store.max_event_id().await.expect("after 3");
    assert_eq!(max, e3.id);
    assert!(max >= e1.id && max >= e2.id);
    assert_eq!(RoomLsn::from_max_id(max).to_header_str(), max.to_string());
    assert!(
        RoomLsn::parse(&max.to_string()).is_some(),
        "header is decimal, not WAL text"
    );
}

#[tokio::test]
async fn max_event_id_is_the_room_head_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn max_event_id_is_the_room_head_postgres() {
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
