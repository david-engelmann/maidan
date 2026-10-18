//! The progress clock behind stuck detection, on both backends: a board read
//! reports when the holder of a live, acknowledged lease last showed progress
//! on the thread, from its acknowledgement, its own messages and its own
//! declaration. Nobody else's message and no lease renewal move it, and an
//! unheld, unacknowledged or lease-less thread has none. Postgres runs under
//! testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::store::ThreadPageQuery;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

pub(super) struct Room {
    pub(super) channel: ChannelId,
    pub(super) worker: MemberId,
    pub(super) lead: MemberId,
}

pub(super) async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for (handle, kind) in [("worker", MemberKind::Agent), ("lead", MemberKind::Human)] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind,
                })
                .await
                .unwrap()
                .id,
        );
    }
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
        channel: channel.id,
        worker: ids[0],
        lead: ids[1],
    }
}

pub(super) async fn task(store: &dyn Store, room: &Room, title: &str) -> ThreadId {
    store
        .create_thread_with_event(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: Some(room.lead),
        })
        .await
        .unwrap()
        .0
        .id
}

async fn post(store: &dyn Store, thread: ThreadId, author: MemberId, body: &str) {
    store
        .post_message(NewMessage {
            thread_id: thread,
            author_id: author,
            body: body.into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
}

/// What both board reads say about the thread's progress; they must agree.
async fn progress(
    store: &dyn Store,
    room: &Room,
    thread: ThreadId,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let read = store.get_thread(thread).await.unwrap().last_progress_at;
    let page = store
        .page_threads_for_channel(
            room.channel,
            ThreadPageQuery {
                after: None,
                limit: 100,
                newest_first: false,
                since: None,
                result_kind: None,
                status: None,
            },
        )
        .await
        .unwrap();
    let paged = page
        .iter()
        .find(|t| t.id == thread)
        .expect("the thread is on its channel's page")
        .last_progress_at;
    assert_eq!(read, paged, "the single read and the page agree");
    read
}

/// Timestamps on SQLite can be second-grained; a step this long always moves
/// a clock that should move.
pub(super) async fn tick() {
    tokio::time::sleep(Duration::from_millis(1100)).await;
}

/// Only the holder's own activity on a live, acknowledged lease moves the
/// clock.
async fn the_holders_own_activity_moves_the_clock(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "progress").await;
    let thread = task(store.as_ref(), &room, "work").await;
    assert_eq!(
        progress(store.as_ref(), &room, thread).await,
        None,
        "unheld"
    );

    let claimed = store
        .claim_next_thread(room.channel, room.worker, Some(600), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, thread);
    assert_eq!(
        progress(store.as_ref(), &room, thread).await,
        None,
        "claimed but not acknowledged is the reaper's ClaimUnacknowledged"
    );
    let lease = claimed.claim_lease_id.unwrap();
    let acked = store
        .acknowledge_claim(thread, room.worker, lease)
        .await
        .unwrap();
    let started = progress(store.as_ref(), &room, thread)
        .await
        .expect("acknowledged");
    assert_eq!(Some(started), acked.work_started_at);

    tick().await;
    post(store.as_ref(), thread, room.lead, "how is it going?").await;
    store
        .renew_claim(thread, room.worker, lease, 600)
        .await
        .unwrap();
    assert_eq!(
        progress(store.as_ref(), &room, thread).await,
        Some(started),
        "another member's message and a renewal are not the holder's progress"
    );

    post(store.as_ref(), thread, room.worker, "halfway").await;
    let posted = progress(store.as_ref(), &room, thread).await.unwrap();
    assert!(posted > started, "the holder's message moves it");

    tick().await;
    store
        .declare_thread_status(
            thread,
            DeclaredStatus::Working,
            "still on it".into(),
            room.worker,
        )
        .await
        .unwrap();
    let declared = progress(store.as_ref(), &room, thread).await.unwrap();
    assert!(declared > posted, "the holder's declaration moves it");

    store
        .set_thread_owner(thread, Some(room.lead))
        .await
        .unwrap();
    tick().await;
    store
        .declare_thread_status(
            thread,
            DeclaredStatus::Working,
            "on it, says the lead".into(),
            room.lead,
        )
        .await
        .unwrap();
    assert_eq!(
        progress(store.as_ref(), &room, thread).await,
        Some(posted),
        "a declaration by someone else replaces the holder's, and is not its progress"
    );
}

/// A lease-less assignment is a person's: it has no progress clock even once
/// acknowledged. A thread that left `open` has none either.
async fn a_lease_less_assignment_or_a_thread_in_review_has_no_clock(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "leaseless").await;
    let thread = task(store.as_ref(), &room, "by hand").await;
    let assigned = store.assign_thread(thread, room.lead).await.unwrap();
    store
        .acknowledge_claim(thread, room.lead, assigned.claim_lease_id.unwrap())
        .await
        .unwrap();
    post(store.as_ref(), thread, room.lead, "doing it").await;
    assert_eq!(progress(store.as_ref(), &room, thread).await, None);

    let leased = task(store.as_ref(), &room, "leased").await;
    let claimed = store
        .claim_next_thread(room.channel, room.worker, Some(600), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, leased);
    store
        .acknowledge_claim(leased, room.worker, claimed.claim_lease_id.unwrap())
        .await
        .unwrap();
    assert!(progress(store.as_ref(), &room, leased).await.is_some());
    store
        .transition_thread(leased, room.worker, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();
    assert_eq!(progress(store.as_ref(), &room, leased).await, None);
}

pub(super) async fn sqlite() -> Arc<dyn Store> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    Arc::new(SqliteStore::for_tests(pool))
}

pub(super) async fn postgres() -> Option<(testcontainers::ContainerAsync<Postgres>, Arc<dyn Store>)>
{
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
    the_holders_own_activity_moves_the_clock,
    a_lease_less_assignment_or_a_thread_in_review_has_no_clock,
);
