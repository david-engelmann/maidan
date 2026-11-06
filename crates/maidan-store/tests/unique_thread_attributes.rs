//! Unique thread attributes on both backends: a workspace that keeps
//! `linear_issue` unique refuses a second open thread on the same ticket, at
//! create and at a later attribute set, naming the holder; a closed or
//! archived holder no longer blocks; another workspace, and a key the
//! workspace does not list, are untouched; and concurrent writers of one
//! ticket leave exactly one winner. Postgres runs under testcontainers
//! (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations, StoreError};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    worker: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let worker = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "worker".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
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
        worker: worker.id,
    }
}

async fn keep_unique(store: &dyn Store, ws: WorkspaceId, keys: Vec<UniqueAttributeKey>) {
    store
        .set_workspace_unique_thread_attributes(
            ws,
            WorkspaceUniqueThreadAttributes { keys: keys.clone() },
            NewAuditEvent {
                scope: AuditScope::Workspace(ws),
                actor_id: None,
                action: "workspace.unique_thread_attributes.set".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(ws.0),
                metadata: serde_json::json!({ "keys": keys }),
            },
        )
        .await
        .unwrap();
}

fn ticket(issue: &str) -> TaskAttributes {
    TaskAttributes {
        linear_issue: Some(issue.into()),
        ..TaskAttributes::default()
    }
}

async fn create(
    store: &dyn Store,
    channel: ChannelId,
    title: &str,
    attributes: Option<TaskAttributes>,
) -> Result<Thread, StoreError> {
    store
        .create_thread_with_footprint(
            NewThread {
                channel_id: channel,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
                created_by: None,
            },
            None,
            attributes,
            None,
        )
        .await
        .map(|(thread, _)| thread)
}

fn held_by(err: StoreError) -> ThreadId {
    match err {
        StoreError::AttributeHeld { key, holder, .. } => {
            assert_eq!(key, UniqueAttributeKey::LinearIssue);
            holder
        }
        other => panic!("expected AttributeHeld, got {other:?}"),
    }
}

async fn close(store: &dyn Store, thread: ThreadId, actor: MemberId) {
    for action in [ThreadAction::StartReview, ThreadAction::Close] {
        store
            .transition_thread(thread, actor, action)
            .await
            .unwrap();
    }
}

/// The setting starts empty, reads back sorted and without repeats, clears,
/// and a missing workspace is NotFound.
async fn the_setting_reads_back_as_set_and_clears(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "setting").await;
    let read = |ws| {
        let store = store.clone();
        async move { store.workspace_unique_thread_attributes(ws).await }
    };
    assert_eq!(
        read(room.ws).await.unwrap().keys,
        Vec::<UniqueAttributeKey>::new()
    );
    keep_unique(
        store.as_ref(),
        room.ws,
        vec![
            UniqueAttributeKey::LinearIssue,
            UniqueAttributeKey::LinearIssue,
        ],
    )
    .await;
    assert_eq!(
        read(room.ws).await.unwrap().keys,
        vec![UniqueAttributeKey::LinearIssue]
    );
    keep_unique(store.as_ref(), room.ws, vec![]).await;
    assert_eq!(
        read(room.ws).await.unwrap().keys,
        Vec::<UniqueAttributeKey>::new()
    );
    assert!(matches!(
        read(WorkspaceId::new()).await,
        Err(StoreError::NotFound)
    ));
}

/// A second open thread on a held ticket is refused, names the holder, and
/// creates nothing.
async fn a_held_ticket_refuses_a_second_thread_and_names_the_holder(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "held").await;
    keep_unique(
        store.as_ref(),
        room.ws,
        vec![UniqueAttributeKey::LinearIssue],
    )
    .await;
    let first = create(
        store.as_ref(),
        room.channel,
        "pi draft",
        Some(ticket("ENG-6598")),
    )
    .await
    .unwrap();
    let refused = create(
        store.as_ref(),
        room.channel,
        "laptop session",
        Some(ticket("ENG-6598")),
    )
    .await
    .expect_err("the ticket is held");
    assert!(refused.to_string().contains("ENG-6598"), "{refused}");
    assert_eq!(held_by(refused), first.id);
    let titles: Vec<_> = store
        .list_threads(room.channel)
        .await
        .unwrap()
        .into_iter()
        .map(|thread| thread.title)
        .collect();
    assert_eq!(
        titles,
        vec![Some("pi draft".to_string())],
        "nothing was created"
    );

    // Another ticket, and a thread with no ticket at all, are not held.
    create(store.as_ref(), room.channel, "other", Some(ticket("ENG-1")))
        .await
        .unwrap();
    create(store.as_ref(), room.channel, "untagged", None)
        .await
        .unwrap();
    create(
        store.as_ref(),
        room.channel,
        "described",
        Some(TaskAttributes::default()),
    )
    .await
    .unwrap();
}

/// A closed holder, or an archived one, holds nothing: the ticket can be
/// reworked.
async fn a_closed_or_archived_holder_no_longer_blocks(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "closed").await;
    keep_unique(
        store.as_ref(),
        room.ws,
        vec![UniqueAttributeKey::LinearIssue],
    )
    .await;
    let first = create(store.as_ref(), room.channel, "first", Some(ticket("ENG-2")))
        .await
        .unwrap();
    close(store.as_ref(), first.id, room.worker).await;
    let rework = create(
        store.as_ref(),
        room.channel,
        "rework",
        Some(ticket("ENG-2")),
    )
    .await
    .expect("a closed thread holds nothing");
    close(store.as_ref(), rework.id, room.worker).await;
    store
        .transition_thread(rework.id, room.worker, ThreadAction::Archive)
        .await
        .unwrap();
    let again = create(store.as_ref(), room.channel, "again", Some(ticket("ENG-2")))
        .await
        .expect("an archived thread holds nothing");
    let refused = create(store.as_ref(), room.channel, "twice", Some(ticket("ENG-2")))
        .await
        .expect_err("the open rework holds it");
    assert_eq!(held_by(refused), again.id);
}

/// One workspace's held ticket never refuses another workspace's thread, and
/// a workspace that keeps nothing unique allows two open threads on one
/// ticket.
async fn another_workspace_is_not_affected(store: Arc<dyn Store>) {
    let mine = room(store.as_ref(), "mine").await;
    let theirs = room(store.as_ref(), "theirs").await;
    let lax = room(store.as_ref(), "lax").await;
    keep_unique(
        store.as_ref(),
        mine.ws,
        vec![UniqueAttributeKey::LinearIssue],
    )
    .await;
    keep_unique(
        store.as_ref(),
        theirs.ws,
        vec![UniqueAttributeKey::LinearIssue],
    )
    .await;
    create(store.as_ref(), mine.channel, "mine", Some(ticket("ENG-3")))
        .await
        .unwrap();
    create(
        store.as_ref(),
        theirs.channel,
        "theirs",
        Some(ticket("ENG-3")),
    )
    .await
    .expect("another workspace's holder does not count");
    for title in ["one", "two"] {
        create(store.as_ref(), lax.channel, title, Some(ticket("ENG-3")))
            .await
            .expect("a workspace that keeps nothing unique");
    }
}

/// Setting attributes later is checked the same way: naming a held ticket is
/// refused and leaves the attributes as they were; restating its own ticket
/// is fine; and a closed thread may be given any ticket.
async fn setting_attributes_later_is_checked(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "later").await;
    keep_unique(
        store.as_ref(),
        room.ws,
        vec![UniqueAttributeKey::LinearIssue],
    )
    .await;
    let holder = create(
        store.as_ref(),
        room.channel,
        "holder",
        Some(ticket("ENG-4")),
    )
    .await
    .unwrap();
    let other = create(store.as_ref(), room.channel, "other", None)
        .await
        .unwrap();
    let refused = store
        .set_thread_attributes(other.id, ticket("ENG-4"))
        .await
        .expect_err("the ticket is held");
    assert_eq!(held_by(refused), holder.id);
    assert_eq!(
        store
            .get_thread_attributes(other.id)
            .await
            .unwrap()
            .linear_issue,
        None,
        "the refused set wrote nothing"
    );
    let restated = TaskAttributes {
        engine: Some(TaskEngine::Pi),
        ..ticket("ENG-4")
    };
    store
        .set_thread_attributes(holder.id, restated)
        .await
        .expect("the holder restates its own ticket");
    store
        .set_thread_attributes(other.id, ticket("ENG-5"))
        .await
        .expect("a free ticket");

    let done = create(store.as_ref(), room.channel, "done", None)
        .await
        .unwrap();
    close(store.as_ref(), done.id, room.worker).await;
    store
        .set_thread_attributes(done.id, ticket("ENG-4"))
        .await
        .expect("a closed thread holds nothing, so labelling it is not refused");
}

/// Concurrent creates on one ticket leave exactly one thread, and so do
/// concurrent sets.
async fn concurrent_writers_leave_exactly_one_winner(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "race").await;
    keep_unique(
        store.as_ref(),
        room.ws,
        vec![UniqueAttributeKey::LinearIssue],
    )
    .await;
    let creates = (0..8).map(|n| {
        let store = store.clone();
        let channel = room.channel;
        tokio::spawn(async move {
            create(
                store.as_ref(),
                channel,
                &format!("racer {n}"),
                Some(ticket("ENG-6")),
            )
            .await
        })
    });
    let outcomes: Vec<_> = futures::future::join_all(creates)
        .await
        .into_iter()
        .map(|joined| joined.unwrap())
        .collect();
    let winners: Vec<&Thread> = outcomes.iter().filter_map(|o| o.as_ref().ok()).collect();
    assert_eq!(winners.len(), 1, "{outcomes:?}");
    let winner = winners[0].id;
    for outcome in outcomes {
        if let Err(err) = outcome {
            assert_eq!(held_by(err), winner);
        }
    }

    let mut blank = Vec::new();
    for n in 0..6 {
        blank.push(
            create(store.as_ref(), room.channel, &format!("blank {n}"), None)
                .await
                .unwrap()
                .id,
        );
    }
    let sets = blank.iter().map(|thread| {
        let store = store.clone();
        let thread = *thread;
        tokio::spawn(async move { store.set_thread_attributes(thread, ticket("ENG-7")).await })
    });
    let outcomes: Vec<_> = futures::future::join_all(sets)
        .await
        .into_iter()
        .map(|joined| joined.unwrap())
        .collect();
    let won = outcomes.iter().filter(|o| o.is_ok()).count();
    assert_eq!(won, 1, "{outcomes:?}");
    for outcome in outcomes {
        if let Err(err) = outcome {
            held_by(err);
        }
    }
}

/// With `linear_issue` kept unique, other attributes may repeat freely, and
/// with nothing listed the ticket may repeat too.
async fn a_key_not_in_the_list_is_unaffected(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "unlisted").await;
    let same_lane = |issue: &str| TaskAttributes {
        engine: Some(TaskEngine::Pi),
        lane: Some(TaskLane::Batch),
        acceptance: Some("It passes.".into()),
        ..ticket(issue)
    };
    for title in ["a", "b"] {
        create(
            store.as_ref(),
            room.channel,
            title,
            Some(same_lane("ENG-8")),
        )
        .await
        .expect("nothing is kept unique yet");
    }
    keep_unique(
        store.as_ref(),
        room.ws,
        vec![UniqueAttributeKey::LinearIssue],
    )
    .await;
    create(store.as_ref(), room.channel, "c", Some(same_lane("ENG-9")))
        .await
        .expect("the same engine, lane and acceptance on another ticket");
    create(store.as_ref(), room.channel, "d", Some(same_lane("ENG-10")))
        .await
        .expect("still another ticket");
    // Turning the rule on leaves the duplicates it finds open; the next
    // write on that ticket is what it refuses.
    let refused = create(store.as_ref(), room.channel, "e", Some(same_lane("ENG-8")))
        .await
        .expect_err("ENG-8 is held");
    held_by(refused);
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

async fn sqlite() -> Arc<dyn Store> {
    let pool = maidan_search::sqlite_pool_options()
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
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some((container, Arc::new(PostgresStore::for_tests(pool))))
}

on_both_backends!(
    the_setting_reads_back_as_set_and_clears,
    a_held_ticket_refuses_a_second_thread_and_names_the_holder,
    a_closed_or_archived_holder_no_longer_blocks,
    another_workspace_is_not_affected,
    setting_attributes_later_is_checked,
    concurrent_writers_leave_exactly_one_winner,
    a_key_not_in_the_list_is_unaffected,
);
