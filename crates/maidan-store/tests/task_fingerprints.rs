//! Task fingerprints (Context Economics C8) on both backends: stored in the
//! create transaction, linked to the oldest open identical task in the same
//! workspace, never across workspaces, and never to a closed task. A change
//! thread for a Linear issue that another change thread has in review is
//! withheld as a likely duplicate.
//! Postgres runs under testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

struct Room {
    channel: ChannelId,
    agents: Vec<MemberId>,
}

async fn room(store: &dyn Store, name: &str, agents: usize) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for n in 0..agents {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: format!("agent{n}"),
                    display_name: None,
                    kind: MemberKind::Agent,
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
        agents: ids,
    }
}

fn new_thread(channel: ChannelId, title: &str) -> NewThread {
    NewThread {
        channel_id: channel,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: None,
    }
}

async fn create(
    store: &dyn Store,
    channel: ChannelId,
    title: &str,
    attributes: Option<TaskAttributes>,
) -> ThreadId {
    store
        .create_thread_with_footprint(new_thread(channel, title), None, attributes, None)
        .await
        .unwrap()
        .0
        .id
}

/// A second identical task is linked to the first; the first links to
/// nothing; the stored fingerprint is the one the types compute.
async fn an_identical_open_task_is_linked_to_the_first(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "dupes", 1).await;
    let acceptance = TaskAttributes {
        acceptance: Some("the sweep closes it".into()),
        ..TaskAttributes::default()
    };
    let first = create(
        store.as_ref(),
        room.channel,
        "Fix the sweep",
        Some(acceptance.clone()),
    )
    .await;
    let second = create(
        store.as_ref(),
        room.channel,
        "  Fix the sweep ",
        Some(acceptance.clone()),
    )
    .await;
    let third = create(
        store.as_ref(),
        room.channel,
        "Fix the sweep",
        Some(acceptance.clone()),
    )
    .await;
    let a = store.get_task_fingerprint(first).await.unwrap().unwrap();
    assert_eq!(
        Some(a.fingerprint.clone()),
        task_fingerprint(Some("Fix the sweep"), None, &acceptance)
    );
    assert_eq!(a.duplicate_of, None);
    let b = store.get_task_fingerprint(second).await.unwrap().unwrap();
    assert_eq!(b.fingerprint, a.fingerprint);
    assert_eq!(b.duplicate_of, Some(first));
    // Linked to the oldest, not the latest.
    let c = store.get_task_fingerprint(third).await.unwrap().unwrap();
    assert_eq!(c.duplicate_of, Some(first));
}

/// A different brief, or the same brief with different inputs, is not a
/// duplicate.
async fn a_different_brief_or_input_is_not_linked(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "near-misses", 1).await;
    create(store.as_ref(), room.channel, "Fix the sweep", None).await;
    let other_title = create(store.as_ref(), room.channel, "Fix the sweeps", None).await;
    let other_input = create(
        store.as_ref(),
        room.channel,
        "Fix the sweep",
        Some(TaskAttributes {
            linear_issue: Some("ENG-6004".into()),
            ..TaskAttributes::default()
        }),
    )
    .await;
    for id in [other_title, other_input] {
        assert_eq!(
            store
                .get_task_fingerprint(id)
                .await
                .unwrap()
                .unwrap()
                .duplicate_of,
            None
        );
    }
}

/// The same brief in another workspace is never linked, though the
/// fingerprint itself is the same.
async fn another_workspaces_identical_task_is_not_linked(store: Arc<dyn Store>) {
    let a = room(store.as_ref(), "tenant-a", 1).await;
    let b = room(store.as_ref(), "tenant-b", 1).await;
    let in_a = create(store.as_ref(), a.channel, "Rotate the keys", None).await;
    let in_b = create(store.as_ref(), b.channel, "Rotate the keys", None).await;
    let fa = store.get_task_fingerprint(in_a).await.unwrap().unwrap();
    let fb = store.get_task_fingerprint(in_b).await.unwrap().unwrap();
    assert_eq!(fa.fingerprint, fb.fingerprint);
    assert_eq!(fb.duplicate_of, None);
}

/// A closed task is not an open duplicate: a new identical task after it
/// links to nothing.
async fn a_closed_task_is_not_linked(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "closed", 1).await;
    let first = create(store.as_ref(), room.channel, "Ship the index", None).await;
    store
        .transition_thread(first, room.agents[0], maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();
    // In review still counts as open work.
    let during = create(store.as_ref(), room.channel, "Ship the index", None).await;
    assert_eq!(
        store
            .get_task_fingerprint(during)
            .await
            .unwrap()
            .unwrap()
            .duplicate_of,
        Some(first)
    );
    store
        .transition_thread(first, room.agents[0], maidan_fsm::ThreadAction::Close)
        .await
        .unwrap();
    store
        .transition_thread(
            during,
            room.agents[0],
            maidan_fsm::ThreadAction::StartReview,
        )
        .await
        .unwrap();
    store
        .transition_thread(during, room.agents[0], maidan_fsm::ThreadAction::Close)
        .await
        .unwrap();
    let after = create(store.as_ref(), room.channel, "Ship the index", None).await;
    assert_eq!(
        store
            .get_task_fingerprint(after)
            .await
            .unwrap()
            .unwrap()
            .duplicate_of,
        None
    );
}

/// A thread with no title or description has no fingerprint.
async fn a_thread_with_no_brief_has_no_fingerprint(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "blank", 1).await;
    let (thread, _) = store
        .create_thread_with_footprint(
            NewThread {
                title: None,
                ..new_thread(room.channel, "x")
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(store.get_task_fingerprint(thread.id).await.unwrap(), None);
}

/// The plain create path stores a fingerprint too, linked like any other:
/// which door a task came in by does not change whether it is a duplicate.
/// A claimed task is still `open` (claims are leases, not a thread state),
/// so in-flight work is a duplicate target.
async fn the_plain_create_path_stores_and_links_a_fingerprint(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "plain", 1).await;
    let first = create(store.as_ref(), room.channel, "Trim the logs", None).await;
    let plain = store
        .create_thread(new_thread(room.channel, "Trim the logs"))
        .await
        .unwrap();
    let stored = store.get_task_fingerprint(plain.id).await.unwrap().unwrap();
    assert_eq!(stored.duplicate_of, Some(first));
}

/// A live duplicate is withheld from claim-next and refused on an explicit
/// claim; the original is handed out instead (C8b).
async fn a_live_duplicate_is_not_claimable(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "dupe-claims", 2).await;
    let original = create(store.as_ref(), room.channel, "Fix the sweep", None).await;
    let duplicate = create(store.as_ref(), room.channel, "Fix the sweep", None).await;
    let err = store
        .claim_thread(duplicate, room.agents[0])
        .await
        .expect_err("a live duplicate is refused");
    assert!(matches!(err, StoreError::Conflict(_)), "{err:?}");
    let first = store
        .claim_next_thread(room.channel, room.agents[0], Some(600), None)
        .await
        .unwrap()
        .expect("the original is claimable");
    assert_eq!(first.id, original);
    // Claimed, the original is still open work, so the duplicate stays
    // withheld.
    assert!(store
        .claim_next_thread(room.channel, room.agents[1], Some(600), None)
        .await
        .unwrap()
        .is_none());
}

/// Clearing the link, or the original closing, makes the duplicate
/// claimable.
async fn a_cleared_or_orphaned_duplicate_is_claimable(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "dupe-release", 1).await;
    let original = create(store.as_ref(), room.channel, "Ship the index", None).await;
    let cleared = create(store.as_ref(), room.channel, "Ship the index", None).await;
    let orphaned = create(store.as_ref(), room.channel, "Ship the index", None).await;
    store.clear_task_duplicate(cleared).await.unwrap();
    assert_eq!(
        store
            .get_task_fingerprint(cleared)
            .await
            .unwrap()
            .unwrap()
            .duplicate_of,
        None
    );
    assert!(
        store
            .claim_thread(cleared, room.agents[0])
            .await
            .unwrap()
            .claimed
    );
    assert!(store.claim_thread(orphaned, room.agents[0]).await.is_err());
    store
        .transition_thread(
            original,
            room.agents[0],
            maidan_fsm::ThreadAction::StartReview,
        )
        .await
        .unwrap();
    assert!(store.claim_thread(orphaned, room.agents[0]).await.is_err());
    store
        .transition_thread(original, room.agents[0], maidan_fsm::ThreadAction::Close)
        .await
        .unwrap();
    assert!(
        store
            .claim_thread(orphaned, room.agents[0])
            .await
            .unwrap()
            .claimed
    );
    // A thread with no fingerprint has no link to clear.
    let (blank, _) = store
        .create_thread_with_footprint(
            NewThread {
                title: None,
                ..new_thread(room.channel, "x")
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        store.clear_task_duplicate(blank.id).await,
        Err(StoreError::NotFound)
    ));
}

fn issue(key: &str) -> Option<TaskAttributes> {
    Some(TaskAttributes {
        linear_issue: Some(key.into()),
        ..TaskAttributes::default()
    })
}

/// Two open change threads for one Linear issue are linked even when their
/// titles differ; the older is the original (C8 follow-up).
async fn the_same_issue_under_another_title_is_linked(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "issue-link", 1).await;
    let first = create(
        store.as_ref(),
        room.channel,
        "Session: ENG-6001 land the board",
        issue("ENG-6001"),
    )
    .await;
    let second = create(
        store.as_ref(),
        room.channel,
        "ENG-6001 land the board",
        issue("ENG-6001"),
    )
    .await;
    let other = create(
        store.as_ref(),
        room.channel,
        "ENG-6002 land the board",
        issue("ENG-6002"),
    )
    .await;
    let review = create(
        store.as_ref(),
        room.channel,
        "Review ENG-6001",
        Some(TaskAttributes {
            kind: TaskKind::Review,
            linear_issue: Some("ENG-6001".into()),
            ..TaskAttributes::default()
        }),
    )
    .await;
    let fp = |id| {
        let store = store.clone();
        async move {
            store
                .get_task_fingerprint(id)
                .await
                .unwrap()
                .unwrap()
                .duplicate_of
        }
    };
    assert_eq!(fp(first).await, None);
    assert_eq!(fp(second).await, Some(first));
    assert_eq!(fp(other).await, None);
    // A review of the issue is not a second attempt at the change.
    assert_eq!(fp(review).await, None);
}

/// A change thread whose issue already has another change thread in review
/// is withheld from claim-next and counted unclaimable, until the other
/// leaves review or someone clears the hold. A review thread for the issue,
/// or another workspace's thread, does not hold it.
async fn a_change_whose_issue_is_in_review_is_withheld(store: Arc<dyn Store>) {
    let home = room(store.as_ref(), "issue-hold", 1).await;
    let agent = home.agents[0];
    // The stale thread is the older one, so no creation-time link holds it;
    // only the in-review rule can.
    let stale = create(
        store.as_ref(),
        home.channel,
        "Session: ENG-6001 land the board",
        issue("ENG-6001"),
    )
    .await;
    let delivered = create(
        store.as_ref(),
        home.channel,
        "ENG-6001 land the board",
        issue("ENG-6001"),
    )
    .await;
    store
        .transition_thread(delivered, agent, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();
    assert!(store
        .claim_next_thread(home.channel, agent, Some(600), None)
        .await
        .unwrap()
        .is_none());
    let depth = store.channel_queue_depth(home.channel, None).await.unwrap();
    assert_eq!((depth.ready, depth.unclaimable), (0, 1), "{depth:?}");

    // Another workspace's thread for the same issue, in review, holds nothing
    // here, and nothing there is linked to this workspace.
    let elsewhere = room(store.as_ref(), "issue-elsewhere", 1).await;
    let theirs = create(
        store.as_ref(),
        elsewhere.channel,
        "ENG-6001 land the board",
        issue("ENG-6001"),
    )
    .await;
    assert_eq!(
        store
            .get_task_fingerprint(theirs)
            .await
            .unwrap()
            .unwrap()
            .duplicate_of,
        None
    );
    let fresh = create(
        store.as_ref(),
        elsewhere.channel,
        "Session: ENG-6001 again",
        issue("ENG-6001"),
    )
    .await;
    store.clear_task_duplicate(fresh).await.unwrap();
    store
        .transition_thread(
            theirs,
            elsewhere.agents[0],
            maidan_fsm::ThreadAction::StartReview,
        )
        .await
        .unwrap();
    // `fresh` was cleared, so their in-review thread does not hold it.
    assert_eq!(
        store
            .claim_next_thread(elsewhere.channel, elsewhere.agents[0], Some(600), None)
            .await
            .unwrap()
            .map(|t| t.id),
        Some(fresh)
    );

    // Clearing the stale thread's hold makes it claimable here.
    store.clear_task_duplicate(stale).await.unwrap();
    assert_eq!(
        store
            .claim_next_thread(home.channel, agent, Some(600), None)
            .await
            .unwrap()
            .map(|t| t.id),
        Some(stale)
    );
}

/// The hold lifts when the other thread leaves review, and a review thread
/// in review for the issue never holds a change.
async fn the_issue_hold_lifts_when_review_ends(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "issue-lift", 1).await;
    let agent = room.agents[0];
    let stale = create(
        store.as_ref(),
        room.channel,
        "Session: ENG-7001 tidy",
        issue("ENG-7001"),
    )
    .await;
    let reviewer = create(
        store.as_ref(),
        room.channel,
        "Review ENG-7001",
        Some(TaskAttributes {
            kind: TaskKind::Review,
            linear_issue: Some("ENG-7001".into()),
            ..TaskAttributes::default()
        }),
    )
    .await;
    store
        .transition_thread(reviewer, agent, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();
    let delivered = create(
        store.as_ref(),
        room.channel,
        "ENG-7001 tidy",
        issue("ENG-7001"),
    )
    .await;
    store.clear_task_duplicate(delivered).await.unwrap();
    store
        .transition_thread(delivered, agent, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();
    assert!(store
        .claim_next_thread(room.channel, agent, Some(600), None)
        .await
        .unwrap()
        .is_none());
    store
        .transition_thread(delivered, agent, maidan_fsm::ThreadAction::Close)
        .await
        .unwrap();
    assert_eq!(
        store
            .claim_next_thread(room.channel, agent, Some(600), None)
            .await
            .unwrap()
            .map(|t| t.id),
        Some(stale)
    );
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
    let pool = SqlitePoolOptions::new()
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

on_both_backends!(
    an_identical_open_task_is_linked_to_the_first,
    a_different_brief_or_input_is_not_linked,
    another_workspaces_identical_task_is_not_linked,
    a_closed_task_is_not_linked,
    a_thread_with_no_brief_has_no_fingerprint,
    the_plain_create_path_stores_and_links_a_fingerprint,
    a_live_duplicate_is_not_claimable,
    a_cleared_or_orphaned_duplicate_is_claimable,
    the_same_issue_under_another_title_is_linked,
    a_change_whose_issue_is_in_review_is_withheld,
    the_issue_hold_lifts_when_review_ends,
);
