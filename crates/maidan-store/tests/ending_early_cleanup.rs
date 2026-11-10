//! What a cancel and a close-abandoned clean up on both backends: each queues
//! the withdrawal of the pull request the change flow opened for the task
//! (and of no other), names the member whose claim it ended on the state
//! change, and the task's result stops counting as an accepted decision. A
//! merge that lands a task someone else holds names that holder too.
//! Postgres runs under testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_fsm::ThreadAction;
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const REPO: &str = "acme/app";

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    producer: MemberId,
    worker: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for handle in ["producer", "worker"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
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
        ws: ws.id,
        channel: channel.id,
        producer: ids[0],
        worker: ids[1],
    }
}

async fn task(store: &dyn Store, room: &Room, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: Some(room.producer),
        })
        .await
        .unwrap()
        .id
}

/// A round the change flow delivered to `pull_number`: what makes it a pull
/// request Maidan opened.
async fn delivered(store: &dyn Store, room: &Room, thread: ThreadId, pull_number: i64) {
    store
        .record_change_round(NewChangeRound {
            workspace_id: room.ws,
            thread_id: thread,
            repo: REPO.into(),
            pull_number,
            commit_sha: format!("{pull_number:0>40}"),
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

fn previous_assignee(events: &[StoredEvent]) -> Option<MemberId> {
    let changed = events
        .iter()
        .find(|e| e.kind == EventKind::ThreadStateChanged)
        .expect("the close appends a state change");
    match serde_json::from_value::<Event>(changed.payload.clone()).unwrap() {
        Event::ThreadStateChanged {
            previous_assignee_id,
            thread,
            ..
        } => {
            assert_eq!(thread.assignee_id, None, "the claim is ended");
            previous_assignee_id
        }
        other => panic!("expected ThreadStateChanged, got {other:?}"),
    }
}

/// The member the close of `thread` took the claim from, from the
/// `ThreadStateChanged` into `closed` among `events`.
fn closed_from(events: &[StoredEvent], thread: ThreadId) -> Option<MemberId> {
    events
        .iter()
        .filter(|e| e.kind == EventKind::ThreadStateChanged && e.thread_id == Some(thread))
        .find_map(
            |e| match serde_json::from_value::<Event>(e.payload.clone()).unwrap() {
                Event::ThreadStateChanged {
                    to_state: ThreadState::Closed,
                    previous_assignee_id,
                    thread,
                    ..
                } => {
                    assert_eq!(thread.assignee_id, None, "the claim is ended");
                    Some(previous_assignee_id)
                }
                _ => None,
            },
        )
        .expect("the landing closes the thread")
}

/// Merge `pull_number` and settle it as the PR sweep does.
async fn merge_and_settle(store: &dyn Store, room: &Room, pull_number: i64) -> Vec<StoredEvent> {
    let at = chrono::Utc::now();
    store
        .close_change_pull(room.ws, REPO, pull_number, Some(at), Some("c0ffee"), at)
        .await
        .unwrap();
    match store
        .settle_change_pull(room.ws, REPO, pull_number)
        .await
        .unwrap()
    {
        ChangeSettlement::Landed(stored) => stored,
        other => panic!("expected the merge to land the task, got {other:?}"),
    }
}

/// A merge lands a task in its owner's name. When the worker still holds it,
/// the close ends that claim, and the state change names the worker, as an
/// ordinary close by someone else does. A task nobody owns lands in the name
/// of the worker that produced its result, who knows already.
async fn a_merge_that_lands_a_held_task_names_its_holder(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "land-names-holder").await;
    let owned = task(store.as_ref(), &room, "owned").await;
    store
        .set_thread_owner(owned, Some(room.producer))
        .await
        .unwrap();
    store
        .set_thread_result_claiming(owned, room.worker, &serde_json::json!({"summary": "done"}))
        .await
        .unwrap();
    delivered(store.as_ref(), &room, owned, 21).await;
    let stored = merge_and_settle(store.as_ref(), &room, 21).await;
    assert_eq!(closed_from(&stored, owned), Some(room.worker));

    let unowned = task(store.as_ref(), &room, "unowned").await;
    store
        .set_thread_result_claiming(
            unowned,
            room.worker,
            &serde_json::json!({"summary": "done"}),
        )
        .await
        .unwrap();
    delivered(store.as_ref(), &room, unowned, 22).await;
    let stored = merge_and_settle(store.as_ref(), &room, 22).await;
    assert_eq!(closed_from(&stored, unowned), None, "the holder closed it");
}

/// A cancel queues one withdrawal, for the open pull request the change flow
/// opened for the task: not a session's pull request tracked on it, and not
/// one already closed. The state change names the worker that held it.
async fn a_cancel_queues_the_withdrawal_of_the_pull_request_the_change_flow_opened(
    store: Arc<dyn Store>,
) {
    let room = room(store.as_ref(), "cancel-withdraw").await;
    let thread = task(store.as_ref(), &room, "port the bridge").await;
    store
        .claim_next_thread(room.channel, room.worker, Some(600), None)
        .await
        .unwrap()
        .expect("the worker claims it");
    delivered(store.as_ref(), &room, thread, 7).await;
    store
        .register_change_pull(room.ws, thread, REPO, 8, None)
        .await
        .unwrap();
    delivered(store.as_ref(), &room, thread, 9).await;
    store
        .close_change_pull(room.ws, REPO, 9, None, None, chrono::Utc::now())
        .await
        .unwrap();

    let events = match store
        .cancel_thread(
            thread,
            room.producer,
            "the bridge is gone",
            CancelAuthority::Creator,
        )
        .await
        .unwrap()
    {
        CancelOutcome::Cancelled { events, .. } => events,
        other => panic!("expected a cancel, got {other:?}"),
    };
    assert_eq!(previous_assignee(&events), Some(room.worker));

    let queued = drain_outbox(store.as_ref()).await;
    assert_eq!(queued.len(), 1, "one withdrawal, for #7 only: {queued:?}");
    let row = &queued[0];
    assert_eq!(row.kind, EgressKind::Withdraw);
    assert_eq!(row.thread_id, thread);
    assert_eq!(row.workspace_id, room.ws);
    assert_eq!(
        row.target(),
        Some(EgressTarget::Github {
            repo: REPO.into(),
            issue_number: 7
        })
    );
    assert!(
        row.body.contains("cancelled: the bridge is gone"),
        "the comment names the reason: {}",
        row.body
    );
    assert!(row.body.contains("branch is left"), "{}", row.body);

    // A repeat cancel changes nothing, so it owes nothing more.
    store
        .cancel_thread(thread, room.producer, "again", CancelAuthority::Creator)
        .await
        .unwrap();
    assert!(drain_outbox(store.as_ref()).await.is_empty());
}

/// A close-abandoned queues the same withdrawal, with its reason, and names
/// the former holder.
async fn a_close_abandoned_queues_the_withdrawal_and_names_the_former_holder(
    store: Arc<dyn Store>,
) {
    let room = room(store.as_ref(), "abandon-withdraw").await;
    let thread = task(store.as_ref(), &room, "nobody will finish this").await;
    store
        .claim_next_thread(room.channel, room.worker, Some(600), None)
        .await
        .unwrap()
        .expect("the worker claims it");
    delivered(store.as_ref(), &room, thread, 11).await;

    let (_, events) = store
        .close_abandoned_thread(
            thread,
            room.producer,
            false,
            &CloseAbandoned {
                reason: CloseReason::Abandoned,
                duplicate_of: None,
            },
            NewAuditEvent {
                scope: AuditScope::Workspace(room.ws),
                actor_id: Some(room.producer),
                action: "thread.close_abandoned".into(),
                target_kind: Some("thread".into()),
                target_id: Some(thread.0),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
    assert_eq!(previous_assignee(&events), Some(room.worker));

    let queued = drain_outbox(store.as_ref()).await;
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_eq!(queued[0].kind, EgressKind::Withdraw);
    assert_eq!(queued[0].selector, format!("{REPO}#11"));
    assert!(
        queued[0].body.contains("closed as abandoned"),
        "{}",
        queued[0].body
    );
}

/// A cancel of a task nobody held names nobody, and a task whose only pull
/// request is a session's queues nothing.
async fn an_unheld_task_names_no_holder_and_a_sessions_pull_request_is_not_withdrawn(
    store: Arc<dyn Store>,
) {
    let room = room(store.as_ref(), "unheld").await;
    let thread = task(store.as_ref(), &room, "a session's work").await;
    store
        .register_change_pull(room.ws, thread, REPO, 21, None)
        .await
        .unwrap();
    let events = match store
        .cancel_thread(thread, room.producer, "dropped", CancelAuthority::Creator)
        .await
        .unwrap()
    {
        CancelOutcome::Cancelled { events, .. } => events,
        other => panic!("expected a cancel, got {other:?}"),
    };
    assert_eq!(previous_assignee(&events), None);
    assert!(drain_outbox(store.as_ref()).await.is_empty());
}

/// The channel's accepted decisions keep a landed result and leave out a
/// cancelled one and an abandoned one.
async fn a_cancelled_or_abandoned_result_is_not_an_accepted_decision(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "decisions").await;
    let landed = task(store.as_ref(), &room, "landed").await;
    store
        .set_thread_result_claiming(landed, room.worker, &serde_json::json!({"decision": "a"}))
        .await
        .unwrap();
    store
        .transition_thread(landed, room.worker, ThreadAction::StartReview)
        .await
        .unwrap();
    store
        .transition_thread(landed, room.worker, ThreadAction::Close)
        .await
        .unwrap();

    let cancelled = task(store.as_ref(), &room, "cancelled").await;
    store
        .set_thread_result_claiming(
            cancelled,
            room.worker,
            &serde_json::json!({"decision": "b"}),
        )
        .await
        .unwrap();
    store
        .cancel_thread(
            cancelled,
            room.producer,
            "not wanted",
            CancelAuthority::Creator,
        )
        .await
        .unwrap();

    let abandoned = task(store.as_ref(), &room, "abandoned").await;
    store
        .set_thread_result_claiming(
            abandoned,
            room.worker,
            &serde_json::json!({"decision": "c"}),
        )
        .await
        .unwrap();
    store
        .close_abandoned_thread(
            abandoned,
            room.producer,
            false,
            &CloseAbandoned {
                reason: CloseReason::Abandoned,
                duplicate_of: None,
            },
            NewAuditEvent {
                scope: AuditScope::Workspace(room.ws),
                actor_id: Some(room.producer),
                action: "thread.close_abandoned".into(),
                target_kind: Some("thread".into()),
                target_id: Some(abandoned.0),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

    let listed: Vec<ThreadId> = store
        .list_channel_closed_results(room.channel, None, 50)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.thread_id)
        .collect();
    assert_eq!(listed, vec![landed]);
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
    a_cancel_queues_the_withdrawal_of_the_pull_request_the_change_flow_opened,
    a_close_abandoned_queues_the_withdrawal_and_names_the_former_holder,
    an_unheld_task_names_no_holder_and_a_sessions_pull_request_is_not_withdrawn,
    a_cancelled_or_abandoned_result_is_not_an_accepted_decision,
    a_merge_that_lands_a_held_task_names_its_holder,
);
