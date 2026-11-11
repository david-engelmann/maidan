//! A superseded task hands its pull request to its successor, on both
//! backends: closing a thread as `superseded` with `duplicate_of`, or
//! creating a thread that `supersedes` it, moves its open tracked pull
//! requests to the successor and queues no withdrawal, so the successor's
//! next round reaches the same pull request and its own cancel withdraws it.
//! Every other ending still withdraws. A successor that cannot take the pull
//! request refuses the close with nothing changed, and nothing crosses a
//! workspace. Postgres runs under testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::store::Supersede;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::UniqueAttributeKey::{LinearIssue, Repo};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const REPO: &str = "acme/app";

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    producer: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let producer = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "producer".into(),
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
        producer: producer.id,
    }
}

fn new_thread(room: &Room, title: &str) -> NewThread {
    NewThread {
        channel_id: room.channel,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: Some(room.producer),
    }
}

async fn task(store: &dyn Store, room: &Room, title: &str) -> ThreadId {
    store
        .create_thread(new_thread(room, title))
        .await
        .unwrap()
        .id
}

/// The change flow delivered a round to `pull_number` for `thread`, and the
/// result delivery recorded it there: Maidan opened the pull request for the
/// task, and the task's delivery answers for it.
async fn delivered(store: &dyn Store, room: &Room, thread: ThreadId, pull_number: i64) {
    let sha = format!("{pull_number:0>40}");
    store
        .record_change_round(NewChangeRound {
            workspace_id: room.ws,
            thread_id: thread,
            repo: REPO.into(),
            pull_number,
            commit_sha: sha.clone(),
            base_sha: "b".repeat(40),
            base_behind: None,
            diff_lines: 3,
            paths: vec!["src/lib.rs".into()],
            base_branch: Some("dev".into()),
            rebase: false,
            previous_head: None,
            linear_issue: None,
        })
        .await
        .unwrap();
    let target = EgressTarget::GithubBranch {
        repo: REPO.into(),
        branch: format!("feature/agent-{pull_number}"),
    };
    let row = store
        .arm_result_delivery(thread, &target, chrono::Utc::now())
        .await
        .unwrap()
        .unwrap();
    store
        .mark_result_delivered(
            row.id,
            Some(&format!("{sha}#{pull_number}")),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
}

/// Every delivery queued so far, claimed so the test can read it.
async fn drain_outbox(store: &dyn Store) -> Vec<EgressOutbox> {
    let mut rows = Vec::new();
    // Ahead of now: the Postgres row is stamped by the container's clock.
    let due = chrono::Utc::now() + chrono::Duration::seconds(5);
    while let Some(row) = store.claim_next_due_egress(due, 120).await.unwrap() {
        rows.push(row);
    }
    rows
}

fn audit(room: &Room, thread: ThreadId) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(room.ws),
        actor_id: Some(room.producer),
        action: "thread.close_abandoned".into(),
        target_kind: Some("thread".into()),
        target_id: Some(thread.0),
        metadata: serde_json::json!({}),
    }
}

async fn close(
    store: &dyn Store,
    room: &Room,
    thread: ThreadId,
    reason: CloseReason,
    duplicate_of: Option<ThreadId>,
) -> Result<(ThreadTransitionResult, Vec<StoredEvent>), StoreError> {
    store
        .close_abandoned_thread(
            thread,
            room.producer,
            false,
            &CloseAbandoned {
                reason,
                duplicate_of,
            },
            audit(room, thread),
        )
        .await
}

fn supersede(room: &Room, old: ThreadId) -> Supersede {
    Supersede {
        thread_id: old,
        actor_id: room.producer,
        maintainer: false,
        audit: audit(room, old),
    }
}

/// The `pull_handed_off` notices on `thread`, as (repo, pull number).
async fn handoff_notices(store: &dyn Store, thread: ThreadId) -> Vec<(String, i64)> {
    store
        .list_messages_after(thread, None, 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.metadata["notice"] == "pull_handed_off")
        .map(|m| {
            (
                m.metadata["repo"].as_str().unwrap().to_string(),
                m.metadata["pull_number"].as_i64().unwrap(),
            )
        })
        .collect()
}

fn withdrawn(rows: &[EgressOutbox]) -> Vec<(ThreadId, String)> {
    rows.iter()
        .filter(|r| r.kind == EgressKind::Withdraw)
        .map(|r| (r.thread_id, r.selector.clone()))
        .collect()
}

/// The fix: closing a task as superseded by a live successor moves its open
/// pull request to the successor and withdraws nothing. The successor
/// answers for it from then on, though only the superseded task ever
/// delivered to it, so the successor's cancel withdraws it.
async fn a_superseded_close_with_a_successor_hands_its_pull_request_on_and_withdraws_nothing(
    store: Arc<dyn Store>,
) {
    let room = room(store.as_ref(), "handoff").await;
    let old = task(store.as_ref(), &room, "round 4").await;
    let successor = task(store.as_ref(), &room, "round 5").await;
    delivered(store.as_ref(), &room, old, 7).await;
    assert_eq!(
        store.change_pull_thread(room.ws, REPO, 7).await.unwrap(),
        Some(old)
    );

    close(
        store.as_ref(),
        &room,
        old,
        CloseReason::Superseded,
        Some(successor),
    )
    .await
    .unwrap();

    assert!(
        withdrawn(&drain_outbox(store.as_ref()).await).is_empty(),
        "a handed-off pull request is not withdrawn"
    );
    assert_eq!(
        store.tracked_pull_thread(room.ws, REPO, 7).await.unwrap(),
        Some(successor),
        "the change pull row moved"
    );
    assert_eq!(
        store.change_pull_thread(room.ws, REPO, 7).await.unwrap(),
        Some(successor),
        "the superseded task's earlier delivery no longer answers for it"
    );
    let notice = vec![(REPO.to_string(), 7)];
    assert_eq!(handoff_notices(store.as_ref(), old).await, notice);
    assert_eq!(handoff_notices(store.as_ref(), successor).await, notice);
    let audited = store.list_audit_for_workspace(room.ws, 50).await.unwrap();
    let handoff = audited
        .iter()
        .find(|a| a.action == "thread.pull_handed_off")
        .expect("the handoff is audited");
    assert_eq!(handoff.metadata["pull_number"], 7);
    assert_eq!(handoff.metadata["to_thread_id"], successor.0.to_string());
    let closed = audited
        .iter()
        .find(|a| a.action == "thread.close_abandoned")
        .expect("the close is audited");
    assert_eq!(
        closed.metadata["handed_pulls"],
        serde_json::json!([{ "repo": REPO, "pull_number": 7 }])
    );

    match store
        .cancel_thread(
            successor,
            room.producer,
            "not needed",
            CancelAuthority::Creator,
        )
        .await
        .unwrap()
    {
        CancelOutcome::Cancelled { .. } => {}
        other => panic!("expected a cancel, got {other:?}"),
    }
    assert_eq!(
        withdrawn(&drain_outbox(store.as_ref()).await),
        vec![(successor, format!("{REPO}#7"))],
        "the successor's cancel withdraws the pull request it took"
    );
}

/// Superseded with no successor, abandoned, duplicate and a cancel each
/// still queue their own task's withdrawal, and move nothing.
async fn every_other_ending_still_withdraws_the_pull_request(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "withdraw").await;
    let other = task(store.as_ref(), &room, "the original").await;
    let superseded = task(store.as_ref(), &room, "superseded alone").await;
    let abandoned = task(store.as_ref(), &room, "abandoned").await;
    let duplicate = task(store.as_ref(), &room, "duplicate").await;
    let cancelled = task(store.as_ref(), &room, "cancelled").await;
    delivered(store.as_ref(), &room, superseded, 31).await;
    delivered(store.as_ref(), &room, abandoned, 32).await;
    delivered(store.as_ref(), &room, duplicate, 33).await;
    delivered(store.as_ref(), &room, cancelled, 34).await;

    close(
        store.as_ref(),
        &room,
        superseded,
        CloseReason::Superseded,
        None,
    )
    .await
    .unwrap();
    close(
        store.as_ref(),
        &room,
        abandoned,
        CloseReason::Abandoned,
        None,
    )
    .await
    .unwrap();
    close(
        store.as_ref(),
        &room,
        duplicate,
        CloseReason::Duplicate,
        Some(other),
    )
    .await
    .unwrap();
    store
        .cancel_thread(
            cancelled,
            room.producer,
            "dropped",
            CancelAuthority::Creator,
        )
        .await
        .unwrap();

    let mut queued = withdrawn(&drain_outbox(store.as_ref()).await);
    queued.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(
        queued,
        vec![
            (superseded, format!("{REPO}#31")),
            (abandoned, format!("{REPO}#32")),
            (duplicate, format!("{REPO}#33")),
            (cancelled, format!("{REPO}#34")),
        ]
    );
    assert_eq!(
        store.tracked_pull_thread(room.ws, REPO, 33).await.unwrap(),
        Some(duplicate),
        "a duplicate's pull request is its own attempt, so it does not move"
    );
    assert!(handoff_notices(store.as_ref(), other).await.is_empty());
}

/// A successor that already tracks an open pull request in the same
/// repository, or that has itself ended, cannot take another: the close is
/// refused, naming the pull request, and nothing changes.
async fn a_successor_that_cannot_take_the_pull_request_refuses_the_close(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "refuse").await;
    let old = task(store.as_ref(), &room, "round 1").await;
    let busy = task(store.as_ref(), &room, "round 2").await;
    delivered(store.as_ref(), &room, old, 41).await;
    delivered(store.as_ref(), &room, busy, 42).await;

    let err = close(
        store.as_ref(),
        &room,
        old,
        CloseReason::Superseded,
        Some(busy),
    )
    .await
    .unwrap_err();
    match err {
        StoreError::Conflict(message) => {
            assert!(message.contains(&format!("{REPO}#42")), "{message}");
            assert!(message.contains(&format!("{REPO}#41")), "{message}");
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    let ended = task(store.as_ref(), &room, "round 3").await;
    store
        .cancel_thread(ended, room.producer, "gone", CancelAuthority::Creator)
        .await
        .unwrap();
    drain_outbox(store.as_ref()).await;
    let err = close(
        store.as_ref(),
        &room,
        old,
        CloseReason::Superseded,
        Some(ended),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, StoreError::Conflict(m) if m.contains(&format!("{REPO}#41"))),
        "{err:?}"
    );

    assert_eq!(
        store.get_thread(old).await.unwrap().state,
        ThreadState::Open,
        "the refused close left the thread open"
    );
    assert_eq!(store.thread_unlanded_close(old).await.unwrap(), None);
    assert_eq!(
        store.tracked_pull_thread(room.ws, REPO, 41).await.unwrap(),
        Some(old)
    );
    assert_eq!(
        store.tracked_pull_thread(room.ws, REPO, 42).await.unwrap(),
        Some(busy)
    );
    assert!(drain_outbox(store.as_ref()).await.is_empty());
    assert!(handoff_notices(store.as_ref(), old).await.is_empty());
    assert!(store
        .list_references_to(RefSide::Thread, old.0)
        .await
        .unwrap()
        .is_empty());
}

/// Two tenants: a successor in another workspace is refused for a close, and
/// a create that names another workspace's thread as the one it supersedes
/// creates nothing and closes nothing.
async fn nothing_is_superseded_across_workspaces(store: Arc<dyn Store>) {
    let a = room(store.as_ref(), "tenant-a").await;
    let b = room(store.as_ref(), "tenant-b").await;
    let mine = task(store.as_ref(), &a, "a's round").await;
    let theirs = task(store.as_ref(), &b, "b's round").await;
    delivered(store.as_ref(), &a, mine, 51).await;
    delivered(store.as_ref(), &b, theirs, 52).await;

    let err = close(
        store.as_ref(),
        &a,
        mine,
        CloseReason::Superseded,
        Some(theirs),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::InvalidInput(_)), "{err:?}");
    assert_eq!(
        store.tracked_pull_thread(a.ws, REPO, 51).await.unwrap(),
        Some(mine)
    );

    let before = store.list_threads(a.channel).await.unwrap().len();
    let err = store
        .create_thread_superseding(
            new_thread(&a, "a's next round"),
            None,
            None,
            None,
            Supersede {
                thread_id: theirs,
                actor_id: a.producer,
                maintainer: true,
                audit: audit(&a, theirs),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidInput(_)), "{err:?}");
    assert_eq!(store.list_threads(a.channel).await.unwrap().len(), before);
    assert_eq!(
        store.get_thread(theirs).await.unwrap().state,
        ThreadState::Open
    );
    assert_eq!(
        store.tracked_pull_thread(b.ws, REPO, 52).await.unwrap(),
        Some(theirs)
    );
    assert!(drain_outbox(store.as_ref()).await.is_empty());
}

async fn hold_tickets_per_repo(store: &dyn Store, ws: WorkspaceId) {
    let unique = vec![vec![LinearIssue, Repo]];
    store
        .set_workspace_unique_thread_attributes(
            ws,
            WorkspaceUniqueThreadAttributes {
                unique: unique.clone(),
            },
            NewAuditEvent {
                scope: AuditScope::Workspace(ws),
                actor_id: None,
                action: "workspace.unique_thread_attributes.set".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(ws.0),
                metadata: serde_json::json!({ "unique": unique }),
            },
        )
        .await
        .unwrap();
}

fn ticket() -> TaskAttributes {
    TaskAttributes {
        linear_issue: Some("ENG-7".into()),
        repo: Some(REPO.into()),
        ..TaskAttributes::default()
    }
}

/// A follow-up round created with `supersedes` takes the ticket its
/// predecessor holds: in one transaction the old round closes as superseded
/// by the new one and its pull request moves, not withdrawn. Without
/// `supersedes` the held ticket still refuses the create.
async fn a_round_created_to_supersede_the_ticket_holder_takes_its_ticket_and_pull_request(
    store: Arc<dyn Store>,
) {
    let room = room(store.as_ref(), "follow-up").await;
    hold_tickets_per_repo(store.as_ref(), room.ws).await;
    let (old, _) = store
        .create_thread_with_footprint(new_thread(&room, "round 4"), None, Some(ticket()), None)
        .await
        .unwrap();
    delivered(store.as_ref(), &room, old.id, 61).await;

    let held = store
        .create_thread_with_footprint(new_thread(&room, "round 5"), None, Some(ticket()), None)
        .await
        .unwrap_err();
    match held {
        StoreError::AttributeHeld { holder, .. } => assert_eq!(holder, old.id),
        other => panic!("expected the ticket to be held, got {other:?}"),
    }

    let done = store
        .create_thread_superseding(
            new_thread(&room, "round 5"),
            None,
            Some(ticket()),
            None,
            supersede(&room, old.id),
        )
        .await
        .unwrap();
    let successor = done.thread.id;
    assert_eq!(done.superseded.thread.id, old.id);
    assert_eq!(done.superseded.to_state, ThreadState::Closed);
    assert_eq!(done.created.thread_id, Some(successor));
    assert_eq!(
        store
            .thread_unlanded_close(old.id)
            .await
            .unwrap()
            .as_deref(),
        Some("superseded")
    );
    let refs = store
        .list_references_to(RefSide::Thread, old.id.0)
        .await
        .unwrap();
    assert!(
        refs.iter()
            .any(|r| r.src_id == successor.0 && r.relation == RelationKind::Supersedes),
        "{refs:?}"
    );
    assert_eq!(
        store.tracked_pull_thread(room.ws, REPO, 61).await.unwrap(),
        Some(successor)
    );
    assert_eq!(
        store.change_pull_thread(room.ws, REPO, 61).await.unwrap(),
        Some(successor)
    );
    assert!(withdrawn(&drain_outbox(store.as_ref()).await).is_empty());
    assert_eq!(
        handoff_notices(store.as_ref(), successor).await,
        vec![(REPO.to_string(), 61)]
    );
    assert_eq!(
        store
            .get_thread_attributes(successor)
            .await
            .unwrap()
            .linear_issue
            .as_deref(),
        Some("ENG-7")
    );
}

/// A `supersedes` naming a thread that already ended is refused, and nothing
/// is created.
async fn a_round_cannot_supersede_a_closed_thread(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "closed-old").await;
    let old = task(store.as_ref(), &room, "round 1").await;
    store
        .cancel_thread(old, room.producer, "done with", CancelAuthority::Creator)
        .await
        .unwrap();
    let before = store.list_threads(room.channel).await.unwrap().len();
    let err = store
        .create_thread_superseding(
            new_thread(&room, "round 2"),
            None,
            None,
            None,
            supersede(&room, old),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Conflict(_)), "{err:?}");
    assert_eq!(
        store.list_threads(room.channel).await.unwrap().len(),
        before
    );
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
    a_superseded_close_with_a_successor_hands_its_pull_request_on_and_withdraws_nothing,
    every_other_ending_still_withdraws_the_pull_request,
    a_successor_that_cannot_take_the_pull_request_refuses_the_close,
    nothing_is_superseded_across_workspaces,
    a_round_created_to_supersede_the_ticket_holder_takes_its_ticket_and_pull_request,
    a_round_cannot_supersede_a_closed_thread,
);
