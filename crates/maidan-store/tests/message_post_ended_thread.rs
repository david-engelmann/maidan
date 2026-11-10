//! A post held to the thread's liveness (a person's or an agent's) is checked
//! inside the insert's own transaction: refused on an archived thread
//! (`Ended`, the route's 409 `thread-ended`) and a deleted one (`Gone`, 410),
//! taken on a closed one. Identically on both backends.
//!
//! The route used to read the thread and then insert, so an archive that
//! committed in between let a message land on an archived thread. On
//! Postgres the race itself is run: an archive holds the thread row while the
//! post waits for it, then commits, and the post is refused.

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations, StoreError};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// The raw pool, so the test can delete a thread the way a tombstone does.
enum Pool {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
}

impl Pool {
    async fn tombstone(&self, id: ThreadId) {
        let now = Utc::now();
        match self {
            Pool::Sqlite(p) => {
                sqlx::query("UPDATE maidan_threads SET tombstoned_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(id.0)
                    .execute(p)
                    .await
                    .expect("tombstone");
            }
            Pool::Postgres(p) => {
                sqlx::query("UPDATE maidan_threads SET tombstoned_at = $1 WHERE id = $2")
                    .bind(now)
                    .bind(id.0)
                    .execute(p)
                    .await
                    .expect("tombstone");
            }
        }
    }
}

struct Room {
    channel: ChannelId,
    author: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "author".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member")
        .id;
    Room {
        channel: channel.id,
        author,
    }
}

async fn task(store: &dyn Store, room: &Room, title: &str, actions: &[ThreadAction]) -> ThreadId {
    let thread = store
        .create_thread(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: Some(room.author),
        })
        .await
        .expect("thread")
        .id;
    for action in actions {
        store
            .transition_thread(thread, room.author, *action)
            .await
            .expect("transition");
    }
    thread
}

fn message(thread: ThreadId, author: MemberId, body: &str) -> NewMessage {
    NewMessage {
        thread_id: thread,
        author_id: author,
        body: body.into(),
        metadata: serde_json::json!({}),
        content: None,
    }
}

async fn messages(store: &dyn Store, thread: ThreadId) -> usize {
    store
        .list_messages(thread, 50)
        .await
        .expect("messages")
        .len()
}

const CLOSE: &[ThreadAction] = &[ThreadAction::StartReview, ThreadAction::Close];
const ARCHIVE: &[ThreadAction] = &[
    ThreadAction::StartReview,
    ThreadAction::Close,
    ThreadAction::Archive,
];

async fn a_post_is_refused_on_an_archived_or_deleted_thread(store: &dyn Store, pool: &Pool) {
    let room = room(store, "liveness").await;

    let archived = task(store, &room, "archived", ARCHIVE).await;
    let before = messages(store, archived).await;
    let plain = store
        .post_message_to_live_thread(message(archived, room.author, "late"))
        .await;
    assert!(matches!(plain, Err(StoreError::Ended(_))), "{plain:?}");
    let with_event = store
        .post_message_to_live_thread_with_event(message(archived, room.author, "late"), None)
        .await;
    assert!(
        matches!(with_event, Err(StoreError::Ended(_))),
        "{with_event:?}"
    );
    assert_eq!(
        messages(store, archived).await,
        before,
        "nothing was written"
    );

    let deleted = task(store, &room, "deleted", &[]).await;
    pool.tombstone(deleted).await;
    let plain = store
        .post_message_to_live_thread(message(deleted, room.author, "late"))
        .await;
    assert!(matches!(plain, Err(StoreError::Gone(_))), "{plain:?}");
    let with_event = store
        .post_message_to_live_thread_with_event(message(deleted, room.author, "late"), None)
        .await;
    assert!(
        matches!(with_event, Err(StoreError::Gone(_))),
        "{with_event:?}"
    );

    let missing = store
        .post_message_to_live_thread(message(ThreadId::new(), room.author, "nowhere"))
        .await;
    assert!(matches!(missing, Err(StoreError::NotFound)), "{missing:?}");

    // A closed thread and an open one still take messages: people discuss
    // the work after it closes.
    for (thread, what) in [
        (task(store, &room, "closed", CLOSE).await, "closed"),
        (task(store, &room, "open", &[]).await, "open"),
    ] {
        let before = messages(store, thread).await;
        store
            .post_message_to_live_thread(message(thread, room.author, "after"))
            .await
            .unwrap_or_else(|e| panic!("{what}: {e:?}"));
        store
            .post_message_to_live_thread_with_event(message(thread, room.author, "after"), None)
            .await
            .unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert_eq!(messages(store, thread).await, before + 2, "{what}");
    }

    // The store's own notices are not held to it.
    store
        .post_message(message(archived, room.author, "notice"))
        .await
        .expect("an unchecked post still writes");
}

async fn run_suite(store: &dyn Store, pool: &Pool) {
    a_post_is_refused_on_an_archived_or_deleted_thread(store, pool).await;
}

#[tokio::test]
async fn a_post_checks_the_thread_inside_its_transaction_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    run_suite(&store, &Pool::Sqlite(pool)).await;
}

/// The race the check closes: an archive that holds the thread row while the
/// post runs. The post waits for the row and, once the archive commits, reads
/// it archived and writes nothing. Without the row lock the insert does not
/// wait for the archive (it takes only a key-share lock on the thread) and
/// the message lands on the archived thread.
async fn an_archive_that_commits_while_a_post_waits_refuses_the_post(
    store: Arc<PostgresStore>,
    pool: &sqlx::PgPool,
) {
    let room = room(&*store, "race").await;
    let thread = task(&*store, &room, "archived mid-post", CLOSE).await;
    let before = messages(&*store, thread).await;

    let mut archive = pool.begin().await.expect("begin");
    sqlx::query("UPDATE maidan_threads SET state = 'archived' WHERE id = $1")
        .bind(thread.0)
        .execute(&mut *archive)
        .await
        .expect("archive");

    let poster = store.clone();
    let author = room.author;
    let post = tokio::spawn(async move {
        poster
            .post_message_to_live_thread_with_event(message(thread, author, "racing"), None)
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !post.is_finished(),
        "the post waits for the thread row the archive holds"
    );
    archive.commit().await.expect("commit the archive");

    let posted = post.await.expect("join");
    assert!(matches!(posted, Err(StoreError::Ended(_))), "{posted:?}");
    assert_eq!(
        messages(&*store, thread).await,
        before,
        "no message landed on the archived thread"
    );
}

#[tokio::test]
async fn a_post_checks_the_thread_inside_its_transaction_postgres() {
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
    let store = Arc::new(PostgresStore::for_tests(pool.clone()));
    run_suite(&*store, &Pool::Postgres(pool.clone())).await;
    an_archive_that_commits_while_a_post_waits_refuses_the_post(store, &pool).await;
}
