//! A write that names a member names one of the workspace it writes into.
//!
//! Setting a thread's owner or assignee, recording a mention and opening a DM
//! each store a member id. Left to the foreign key, an id that named no member
//! failed as a database error (a 500 over HTTP) and a member of another
//! workspace was stored. Both are now `NotFound`, and nothing is written.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Member, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace, Thread,
    WorkspaceId,
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

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> Member {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member")
}

async fn thread(store: &dyn Store, ws: WorkspaceId) -> Thread {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .expect("thread")
}

fn not_found<T: std::fmt::Debug>(what: &str, result: Result<T, StoreError>) {
    assert!(
        matches!(result, Err(StoreError::NotFound)),
        "{what}: expected NotFound, got {result:?}"
    );
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .expect("ws");
    let other_ws = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .expect("ws b");
    let own = member(store, ws.id, "own").await;
    let peer = member(store, ws.id, "peer").await;
    let foreign = member(store, other_ws.id, "foreign").await;
    let nobody = maidan_types::MemberId(uuid::Uuid::now_v7());
    let t = thread(store, ws.id).await;
    let message = store
        .post_message(NewMessage {
            thread_id: t.id,
            author_id: own.id,
            body: "hello".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("message");
    let events_before = store.max_event_id().await.expect("max event");

    for (who, id) in [("no member", nobody), ("a foreign member", foreign.id)] {
        not_found(
            &format!("owner naming {who}"),
            store.set_thread_owner(t.id, Some(id)).await,
        );
        not_found(
            &format!("assignee naming {who}"),
            store.assign_thread(t.id, id).await,
        );
        not_found(
            &format!("assignment event naming {who}"),
            store.assign_thread_with_event(t.id, id, own.id, None).await,
        );
        not_found(
            &format!("mention of {who}"),
            store.record_mention_with_event(message.id, id).await,
        );
        not_found(
            &format!("DM with {who}"),
            store.open_dm_conversation(ws.id, own.id, id).await,
        );
    }
    let unchanged = store.get_thread(t.id).await.expect("thread");
    assert_eq!(unchanged.owner_id, None);
    assert_eq!(unchanged.assignee_id, None);
    assert!(store
        .list_mentions_for_member(foreign.id, 10)
        .await
        .expect("mentions")
        .is_empty());
    assert!(store
        .list_dm_conversations_for_member(ws.id, own.id)
        .await
        .expect("dms")
        .is_empty());
    assert_eq!(
        store.max_event_id().await.expect("max event"),
        events_before,
        "a refused write appended an event"
    );

    // A member of the workspace is still accepted everywhere.
    let owned = store
        .set_thread_owner(t.id, Some(peer.id))
        .await
        .expect("owner");
    assert_eq!(owned.owner_id, Some(peer.id));
    assert_eq!(
        store
            .set_thread_owner(t.id, None)
            .await
            .expect("clear")
            .owner_id,
        None
    );
    let (assigned, _) = store
        .assign_thread_with_event(t.id, peer.id, own.id, None)
        .await
        .expect("assign");
    assert_eq!(assigned.assignee_id, Some(peer.id));
    store
        .assign_thread(t.id, own.id)
        .await
        .expect("assign without event");
    store
        .record_mention_with_event(message.id, peer.id)
        .await
        .expect("mention");
    store
        .open_dm_conversation(ws.id, own.id, peer.id)
        .await
        .expect("dm");
}

#[tokio::test]
async fn a_member_outside_the_workspace_is_not_found_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_member_outside_the_workspace_is_not_found_postgres() {
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
