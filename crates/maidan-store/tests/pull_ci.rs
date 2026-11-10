//! CI on tracked pull requests on both backends: a read of a pull request's
//! checks is recorded on its thread and owes each notice once per head, a
//! watched pull request gets a thread of its own, stays out of the swarm
//! metrics and closes when the pull request does, and nothing crosses a
//! workspace. Postgres runs under
//! testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::pull_watch::{watch_pull, WatchRequest};
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const REPO: &str = "example/app";

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    member: MemberId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "watcher".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id;
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
        member,
    }
}

async fn task(store: &dyn Store, room: &Room) -> ThreadId {
    let thread = store
        .create_thread(NewThread {
            channel_id: room.channel,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: Some(room.member),
        })
        .await
        .unwrap()
        .id;
    store
        .set_thread_owner(thread, Some(room.member))
        .await
        .unwrap();
    thread
}

fn read(
    ws: WorkspaceId,
    head: &str,
    status: CiStatus,
    failing: &[&str],
    finished: bool,
) -> NewPullCi {
    NewPullCi {
        workspace_id: ws,
        repo: REPO.into(),
        pull_number: 7,
        head_sha: head.into(),
        status,
        failing_jobs: failing.iter().map(|job| job.to_string()).collect(),
        log_tail: (!failing.is_empty()).then(|| "error: test failed".to_string()),
        finished,
        merge_state: None,
        unresolved_threads: 0,
        unresolved_thread_links: Vec::new(),
        missing_checks: Vec::new(),
        gate_reasons: None,
        merge_queue: None,
        head_ahead: false,
        reviews: Vec::new(),
    }
}

/// What holds a pull request back is said once two reads agree on a
/// change, and only then: blocked with the reasons, clear when none are
/// left, nothing for a read that could not say (which keeps the last
/// answer).
async fn a_gate_change_is_said_once(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "gate").await;
    let thread = task(store.as_ref(), &room).await;
    let s = store.as_ref();
    s.register_change_pull(room.ws, thread, REPO, 7, None)
        .await
        .unwrap();
    let gated = |reasons: Option<&[&str]>| NewPullCi {
        merge_state: Some("BLOCKED".into()),
        unresolved_threads: 1,
        unresolved_thread_links: vec![ReviewThreadLink {
            url: "https://example.test/r1".into(),
            author: Some("review-bot[bot]".into()),
        }],
        missing_checks: vec!["e2e".into()],
        gate_reasons: reasons.map(|r| r.iter().map(|x| x.to_string()).collect()),
        ..read(room.ws, "aaa1", CiStatus::Pending, &[], false)
    };
    let gate_kinds = |posted: Vec<PostedNotice>| -> Vec<(ThreadNoticeKind, Vec<String>)> {
        posted
            .into_iter()
            .filter(|p| p.notice.kind != ThreadNoticeKind::PrOpened)
            .map(|p| (p.notice.kind, p.notice.reasons))
            .collect()
    };
    let held = ["1 unresolved review thread", "merge state BLOCKED"];
    assert!(
        gate_kinds(s.record_pull_ci(gated(Some(&held))).await.unwrap()).is_empty(),
        "one read of a block is not yet said"
    );
    assert_eq!(
        gate_kinds(s.record_pull_ci(gated(Some(&held))).await.unwrap()),
        vec![(
            ThreadNoticeKind::GateBlocked,
            held.iter().map(|r| r.to_string()).collect()
        )]
    );
    assert!(gate_kinds(s.record_pull_ci(gated(Some(&held))).await.unwrap()).is_empty());
    assert!(
        gate_kinds(s.record_pull_ci(gated(None)).await.unwrap()).is_empty(),
        "an unread gate says nothing"
    );
    let ci = s.get_pull_ci(room.ws, REPO, 7).await.unwrap().unwrap();
    assert_eq!(
        ci.gate_reasons,
        held.iter().map(|r| r.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(ci.merge_state.as_deref(), Some("BLOCKED"));
    assert_eq!(ci.unresolved_threads, 1);
    assert_eq!(
        ci.unresolved_thread_links[0].author.as_deref(),
        Some("review-bot[bot]")
    );
    assert_eq!(ci.missing_checks, vec!["e2e".to_string()]);
    assert!(gate_kinds(s.record_pull_ci(gated(Some(&[]))).await.unwrap()).is_empty());
    assert_eq!(
        gate_kinds(s.record_pull_ci(gated(Some(&[]))).await.unwrap()),
        vec![(ThreadNoticeKind::GateClear, Vec::new())]
    );
    assert!(gate_kinds(s.record_pull_ci(gated(Some(&held))).await.unwrap()).is_empty());
    assert_eq!(
        gate_kinds(s.record_pull_ci(gated(Some(&held))).await.unwrap()).len(),
        1,
        "held again after a clear is news again"
    );
    let dequeued = MergeQueue {
        state: MergeQueueState::Dequeued,
        reason: Some("MERGE_CONFLICT".into()),
    };
    let posted = s
        .record_pull_ci(NewPullCi {
            merge_queue: Some(dequeued.clone()),
            ..gated(Some(&held))
        })
        .await
        .unwrap();
    assert_eq!(
        gate_kinds(posted),
        vec![(
            ThreadNoticeKind::MergeDequeued,
            vec!["MERGE_CONFLICT".to_string()]
        )]
    );
    assert_eq!(
        s.get_pull_ci(room.ws, REPO, 7)
            .await
            .unwrap()
            .unwrap()
            .merge_queue,
        Some(dequeued)
    );
    // A head once ahead of its base stays so while it is the head, even
    // after the base catches up; a new head starts over.
    for ahead in [true, false] {
        s.record_pull_ci(NewPullCi {
            head_ahead: ahead,
            ..gated(None)
        })
        .await
        .unwrap();
    }
    assert!(s
        .pull_head_was_ahead(room.ws, REPO, 7, "aaa1")
        .await
        .unwrap());
    let other = self::room(s, "other-ahead").await;
    assert!(
        !s.pull_head_was_ahead(other.ws, REPO, 7, "aaa1")
            .await
            .unwrap(),
        "another workspace's read says nothing"
    );
    s.close_change_pull(
        room.ws,
        REPO,
        7,
        Some(chrono::Utc::now()),
        Some("aaa1"),
        chrono::Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        s.list_thread_pull_ci(thread).await.unwrap()[0]
            .merge_queue
            .as_ref()
            .map(|queue| queue.state),
        Some(MergeQueueState::Landed),
        "a recorded merge reads as landed"
    );
    let notices = s.list_thread_notices(thread, 10).await.unwrap();
    let blocked = notices
        .iter()
        .find(|n| n.kind == ThreadNoticeKind::GateBlocked)
        .unwrap();
    assert_eq!(blocked.reasons.len(), 2);
}

/// A merge state that flickers for one read says nothing, and a block that
/// holds across a restart (a new store over the same database) is said
/// once: what was last said lives with the record, not the process.
async fn a_held_block_is_said_once_across_a_restart(before: Arc<dyn Store>, after: Arc<dyn Store>) {
    let room = room(before.as_ref(), "restart").await;
    let thread = task(before.as_ref(), &room).await;
    before
        .register_change_pull(room.ws, thread, REPO, 7, None)
        .await
        .unwrap();
    let gated = |reasons: &[&str]| NewPullCi {
        gate_reasons: Some(reasons.iter().map(|x| x.to_string()).collect()),
        ..read(room.ws, "aaa1", CiStatus::Success, &[], true)
    };
    let blocked = ["merge state BLOCKED"];
    let gate_said = |posted: Vec<PostedNotice>| {
        posted
            .into_iter()
            .filter(|p| {
                matches!(
                    p.notice.kind,
                    ThreadNoticeKind::GateBlocked | ThreadNoticeKind::GateClear
                )
            })
            .count()
    };
    let mut said = 0;
    for reasons in [&[][..], &blocked, &[], &blocked, &blocked] {
        said += gate_said(before.record_pull_ci(gated(reasons)).await.unwrap());
    }
    assert_eq!(said, 1, "the flicker said nothing; the held block once");
    for _ in 0..3 {
        said += gate_said(after.record_pull_ci(gated(&blocked)).await.unwrap());
    }
    assert_eq!(said, 1, "a restart does not say the held block again");
    let notices = after.list_thread_notices(thread, 20).await.unwrap();
    assert_eq!(
        notices
            .iter()
            .filter(|n| n.kind == ThreadNoticeKind::GateBlocked)
            .count(),
        1
    );
}

#[tokio::test]
async fn a_held_block_is_said_once_across_a_restart_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    maidan_store::configure_sqlite_pool(&pool).await.unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    a_held_block_is_said_once_across_a_restart(
        Arc::new(SqliteStore::for_tests(pool.clone())),
        Arc::new(SqliteStore::for_tests(pool)),
    )
    .await;
}

#[tokio::test]
async fn a_held_block_is_said_once_across_a_restart_postgres() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };
    a_held_block_is_said_once_across_a_restart(
        Arc::new(PostgresStore::for_tests(pool.clone())),
        Arc::new(PostgresStore::for_tests(pool)),
    )
    .await;
}

fn kinds(posted: &[PostedNotice]) -> Vec<ThreadNoticeKind> {
    posted.iter().map(|p| p.notice.kind).collect()
}

/// Each read rewrites the thread's CI record; `pr_opened` is said once for
/// the pull request, `ci_failed` and `checks_done` once per head, and a
/// second read of the same head says nothing.
async fn a_ci_read_says_each_notice_once_per_head(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "ci").await;
    let thread = task(store.as_ref(), &room).await;
    store
        .register_change_pull(room.ws, thread, REPO, 7, None)
        .await
        .unwrap();
    let s = store.as_ref();

    let first = s
        .record_pull_ci(read(room.ws, "aaa1", CiStatus::Pending, &[], false))
        .await
        .unwrap();
    assert_eq!(kinds(&first), vec![ThreadNoticeKind::PrOpened]);
    assert_eq!(first[0].author_id, Some(room.member), "the owner says it");
    assert_eq!(first[0].event.kind, EventKind::ThreadNotice);
    assert_eq!(first[0].event.payload["notice"], "pr_opened");
    assert_eq!(first[0].event.payload["head_sha"], "aaa1");
    let again = s
        .record_pull_ci(read(room.ws, "aaa1", CiStatus::Pending, &[], false))
        .await
        .unwrap();
    assert!(
        again.is_empty(),
        "the next tick over the same head is not news"
    );

    let failing = s
        .record_pull_ci(read(
            room.ws,
            "aaa1",
            CiStatus::Failure,
            &["unit tests"],
            false,
        ))
        .await
        .unwrap();
    assert_eq!(kinds(&failing), vec![ThreadNoticeKind::CiFailed]);
    let done = s
        .record_pull_ci(read(
            room.ws,
            "aaa1",
            CiStatus::Failure,
            &["unit tests"],
            true,
        ))
        .await
        .unwrap();
    assert_eq!(kinds(&done), vec![ThreadNoticeKind::ChecksDone]);
    assert!(s
        .record_pull_ci(read(
            room.ws,
            "aaa1",
            CiStatus::Failure,
            &["unit tests"],
            true
        ))
        .await
        .unwrap()
        .is_empty());

    let ci = s.get_pull_ci(room.ws, REPO, 7).await.unwrap().unwrap();
    assert_eq!(ci.head_sha, "aaa1");
    assert_eq!(ci.status, CiStatus::Failure);
    assert_eq!(ci.failing_jobs, vec!["unit tests".to_string()]);
    assert_eq!(ci.log_tail.as_deref(), Some("error: test failed"));

    // A push starts the checks again: the new head is news, the open is not.
    assert!(s
        .record_pull_ci(read(room.ws, "bbb2", CiStatus::Pending, &[], false))
        .await
        .unwrap()
        .is_empty());
    let green = s
        .record_pull_ci(read(room.ws, "bbb2", CiStatus::Success, &[], true))
        .await
        .unwrap();
    assert_eq!(kinds(&green), vec![ThreadNoticeKind::ChecksDone]);

    let on_thread = s.list_thread_pull_ci(thread).await.unwrap();
    assert_eq!(on_thread.len(), 1);
    assert_eq!(on_thread[0].head_sha, "bbb2");
    assert_eq!(on_thread[0].status, CiStatus::Success);
    assert!(on_thread[0].failing_jobs.is_empty() && on_thread[0].log_tail.is_none());

    let notices = s.list_thread_notices(thread, 10).await.unwrap();
    assert_eq!(notices.len(), 4);
    let failed = notices
        .iter()
        .find(|n| n.kind == ThreadNoticeKind::CiFailed)
        .unwrap();
    assert_eq!(failed.head_sha.as_deref(), Some("aaa1"));
    assert_eq!(failed.status, Some(CiStatus::Failure));
    assert_eq!(failed.failing_jobs, vec!["unit tests".to_string()]);
    assert_eq!(notices[0].kind, ThreadNoticeKind::ChecksDone);
    assert_eq!(notices[0].status, Some(CiStatus::Success));
}

/// A submitted review is said once, by its id, naming who reviewed, how it
/// came out and the head it was on; it reads back the same, and the next
/// read carrying it says nothing (#1935).
async fn a_review_is_said_once_with_who_how_and_which_head(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "review").await;
    let thread = task(store.as_ref(), &room).await;
    let s = store.as_ref();
    s.register_change_pull(room.ws, thread, REPO, 7, None)
        .await
        .unwrap();
    let changes = SubmittedReview {
        id: "PRR_1".into(),
        reviewer: Some("review-bot[bot]".into()),
        state: ReviewState::ChangesRequested,
        head_sha: "aaa1".into(),
    };
    let with = |reviews: Vec<SubmittedReview>| NewPullCi {
        reviews,
        ..read(room.ws, "bbb2", CiStatus::Pending, &[], false)
    };
    let first = s.record_pull_ci(with(vec![changes.clone()])).await.unwrap();
    let said: Vec<_> = first
        .iter()
        .filter(|p| p.notice.kind == ThreadNoticeKind::ReviewSubmitted)
        .collect();
    assert_eq!(said.len(), 1);
    assert_eq!(said[0].notice.reviewer.as_deref(), Some("review-bot[bot]"));
    assert_eq!(
        said[0].notice.review_state,
        Some(ReviewState::ChangesRequested)
    );
    assert_eq!(
        said[0].notice.head_sha.as_deref(),
        Some("aaa1"),
        "the head it reviewed"
    );
    assert_eq!(said[0].event.payload["notice"], "review_submitted");
    assert_eq!(said[0].event.payload["reviewer"], "review-bot[bot]");
    assert_eq!(said[0].event.payload["review_state"], "changes_requested");

    let approval = SubmittedReview {
        id: "PRR_2".into(),
        reviewer: Some("octocat".into()),
        state: ReviewState::Approved,
        head_sha: "bbb2".into(),
    };
    let next = s
        .record_pull_ci(with(vec![changes, approval]))
        .await
        .unwrap();
    assert_eq!(
        kinds(&next),
        vec![ThreadNoticeKind::ReviewSubmitted],
        "only the new review is news"
    );
    assert_eq!(next[0].notice.reviewer.as_deref(), Some("octocat"));

    let notices = s.list_thread_notices(thread, 10).await.unwrap();
    let reviews: Vec<_> = notices
        .iter()
        .filter(|n| n.kind == ThreadNoticeKind::ReviewSubmitted)
        .map(|n| (n.reviewer.as_deref(), n.review_state, n.head_sha.as_deref()))
        .collect();
    assert_eq!(
        reviews,
        vec![
            (Some("octocat"), Some(ReviewState::Approved), Some("bbb2")),
            (
                Some("review-bot[bot]"),
                Some(ReviewState::ChangesRequested),
                Some("aaa1")
            ),
        ]
    );
}

/// Watching gives a pull request a thread in the workspace's public
/// `pull-requests` channel, owned by the watcher; watching it again answers
/// the same thread. A watched pull request is no agent task, so the swarm
/// metrics leave it out, while one registered on a task stays in.
async fn watching_makes_one_thread_and_stays_out_of_the_metrics(store: Arc<dyn Store>) {
    let room = room(store.as_ref(), "watch").await;
    let s = store.as_ref();
    let request = |pull_number| WatchRequest {
        workspace_id: room.ws,
        repo: REPO.into(),
        pull_number,
        channel_id: None,
        title: None,
        created_by: Some(room.member),
    };
    let first = watch_pull(s, request(7)).await.unwrap();
    assert!(first.watched.created);
    assert_eq!(
        first.events.len(),
        2,
        "the channel and the thread were made"
    );
    let thread = s.get_thread(first.watched.thread_id).await.unwrap();
    assert_eq!(thread.title.as_deref(), Some("example/app#7"));
    assert_eq!(thread.owner_id, Some(room.member));
    let channel = s.get_channel(thread.channel_id).await.unwrap();
    assert_eq!(channel.name, DEFAULT_WATCH_CHANNEL);
    assert!(!channel.private);

    let again = watch_pull(s, request(7)).await.unwrap();
    assert!(!again.watched.created);
    assert_eq!(again.watched.thread_id, first.watched.thread_id);
    assert!(again.events.is_empty());

    let second = watch_pull(s, request(8)).await.unwrap();
    assert!(second.watched.created);
    assert_eq!(second.events.len(), 1, "the channel is reused");

    let named = watch_pull(
        s,
        WatchRequest {
            channel_id: Some(room.channel),
            title: Some("the release".into()),
            ..request(9)
        },
    )
    .await
    .unwrap();
    let named = s.get_thread(named.watched.thread_id).await.unwrap();
    assert_eq!(named.channel_id, room.channel);
    assert_eq!(named.title.as_deref(), Some("the release"));

    let task = task(s, &room).await;
    s.register_change_pull(room.ws, task, REPO, 10, None)
        .await
        .unwrap();
    let tracked: Vec<i64> = s
        .list_change_pulls(room.ws, None, None, None, None, None)
        .await
        .unwrap()
        .iter()
        .map(|pull| pull.pull_number)
        .collect();
    assert_eq!(tracked, vec![10]);
    let mut open: Vec<i64> = s
        .list_open_change_pulls(room.ws, 25)
        .await
        .unwrap()
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    open.sort_unstable();
    assert_eq!(open, vec![7, 8, 9, 10], "the sweep still reads every one");
}

/// A thread that only watches a pull request closes when the pull request
/// merges and when it closes without merging, with one message naming the
/// pull request and how it ended, said as the watcher. It closes past a
/// review requirement and the land gate: a watch has no result or delivery
/// of its own for them to hold. A thread with a result keeps the change's
/// settle exactly, even when its pull request was only watched: its gates
/// still refuse a merge, and a close without one still leaves it open.
async fn a_watch_closes_when_its_pull_ends_and_a_change_keeps_its_gates(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let room = room(s, "watch-ends").await;
    let watch = |pull_number| WatchRequest {
        workspace_id: room.ws,
        repo: REPO.into(),
        pull_number,
        channel_id: None,
        title: None,
        created_by: Some(room.member),
    };
    let merged = watch_pull(s, watch(21)).await.unwrap().watched.thread_id;
    let unmerged = watch_pull(s, watch(22)).await.unwrap().watched.thread_id;
    s.set_review_requirement(merged, 1).await.unwrap();
    s.require_land_gate(merged).await.unwrap();
    let now = chrono::Utc::now();
    s.close_change_pull(room.ws, REPO, 21, Some(now), Some("aaa1"), now)
        .await
        .unwrap();
    s.close_change_pull(room.ws, REPO, 22, None, None, now)
        .await
        .unwrap();

    let ChangeSettlement::WatchClosed {
        merged: true,
        events,
    } = s.settle_change_pull(room.ws, REPO, 21).await.unwrap()
    else {
        panic!("a merge closes the watch, its gates notwithstanding");
    };
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            EventKind::MessagePosted,
            EventKind::ThreadStateChanged,
            EventKind::ThreadLanded
        ],
        "the message, open straight to closed, then the landing"
    );
    assert!(events.iter().all(|e| e.thread_id == Some(merged)));
    assert_eq!(
        s.get_thread(merged).await.unwrap().state,
        ThreadState::Closed
    );
    let said = s.list_messages(merged, 10).await.unwrap();
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(
        said[0].body,
        "example/app#21 merged, so this watch is closed."
    );
    assert_eq!(said[0].author_id, room.member);
    assert_eq!(said[0].metadata["github"]["closed_unmerged"], false);
    assert!(
        matches!(
            s.settle_change_pull(room.ws, REPO, 21).await.unwrap(),
            ChangeSettlement::Unchanged
        ),
        "a close settles once"
    );

    let ChangeSettlement::WatchClosed {
        merged: false,
        events,
    } = s.settle_change_pull(room.ws, REPO, 22).await.unwrap()
    else {
        panic!("a close without a merge closes the watch too");
    };
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::MessagePosted, EventKind::ThreadStateChanged],
        "nothing landed"
    );
    assert_eq!(
        s.get_thread(unmerged).await.unwrap().state,
        ThreadState::Closed
    );
    let said = s.list_messages(unmerged, 10).await.unwrap();
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(
        said[0].body,
        "example/app#22 closed without merging, so this watch is closed."
    );
    assert_eq!(said[0].metadata["github"]["closed_unmerged"], true);

    // Changes: a result produced by a worker, so the owner may land it and
    // only the gates decide.
    let worker = agent(s, room.ws, "worker").await;
    let result = serde_json::json!({"summary": "done"});
    let gated = task(s, &room).await;
    s.set_thread_result_claiming(gated, worker, &result)
        .await
        .unwrap();
    s.set_review_requirement(gated, 1).await.unwrap();
    s.watch_change_pull(room.ws, gated, REPO, 23).await.unwrap();
    s.close_change_pull(room.ws, REPO, 23, Some(now), Some("bbb1"), now)
        .await
        .unwrap();
    match s.settle_change_pull(room.ws, REPO, 23).await.unwrap() {
        ChangeSettlement::CloseRefused {
            thread_id, reason, ..
        } => {
            assert_eq!(thread_id, gated);
            assert!(reason.starts_with("review requirement not met"), "{reason}");
        }
        other => panic!("a thread with a result keeps its gates, got {other:?}"),
    }
    assert_eq!(s.get_thread(gated).await.unwrap().state, ThreadState::Open);

    let session = task(s, &room).await;
    s.set_thread_result_claiming(session, worker, &result)
        .await
        .unwrap();
    s.register_change_pull(room.ws, session, REPO, 24, None)
        .await
        .unwrap();
    s.close_change_pull(room.ws, REPO, 24, None, None, now)
        .await
        .unwrap();
    match s.settle_change_pull(room.ws, REPO, 24).await.unwrap() {
        ChangeSettlement::ClosedUnmerged { thread_id, .. } => assert_eq!(thread_id, session),
        other => panic!("a change closed unmerged waits for a person, got {other:?}"),
    }
    assert_eq!(
        s.get_thread(session).await.unwrap().state,
        ThreadState::Open
    );
}

/// Two workspaces can track the same pull request: each gets its own thread
/// and its own check notices, and one closing or settling its row leaves the
/// other's untouched. A channel named from another workspace is not found.
async fn two_workspaces_track_the_same_pull_request_apart(store: Arc<dyn Store>) {
    let a = room(store.as_ref(), "a").await;
    let b = room(store.as_ref(), "b").await;
    let s = store.as_ref();
    let thread = task(s, &a).await;
    s.register_change_pull(a.ws, thread, REPO, 7, None)
        .await
        .unwrap();

    let theirs = watch_pull(
        s,
        WatchRequest {
            workspace_id: b.ws,
            repo: REPO.into(),
            pull_number: 7,
            channel_id: None,
            title: None,
            created_by: Some(b.member),
        },
    )
    .await
    .unwrap();
    assert!(theirs.watched.created);
    assert_ne!(theirs.watched.thread_id, thread);
    assert_eq!(
        s.tracked_pull_thread(a.ws, REPO, 7).await.unwrap(),
        Some(thread),
        "the first workspace still tracks its own"
    );
    let foreign_channel = watch_pull(
        s,
        WatchRequest {
            workspace_id: b.ws,
            repo: REPO.into(),
            pull_number: 8,
            channel_id: Some(a.channel),
            title: None,
            created_by: Some(b.member),
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(foreign_channel, StoreError::NotFound),
        "{foreign_channel:?}"
    );

    assert!(!s
        .record_pull_ci(read(b.ws, "aaa1", CiStatus::Failure, &["unit tests"], true))
        .await
        .unwrap()
        .is_empty());
    assert!(s.get_pull_ci(b.ws, REPO, 7).await.unwrap().is_some());
    assert!(
        s.get_pull_ci(a.ws, REPO, 7).await.unwrap().is_none(),
        "the other workspace's read is not this one's"
    );
    assert!(s.list_thread_notices(thread, 10).await.unwrap().is_empty());
    assert!(!s
        .record_pull_ci(read(a.ws, "aaa1", CiStatus::Failure, &["unit tests"], true))
        .await
        .unwrap()
        .is_empty());
    assert!(!s.list_thread_notices(thread, 10).await.unwrap().is_empty());

    let now = chrono::Utc::now();
    s.close_change_pull(a.ws, REPO, 7, Some(now), Some("aaa1"), now)
        .await
        .unwrap();
    assert!(s.list_open_change_pulls(a.ws, 25).await.unwrap().is_empty());
    assert_eq!(
        s.list_open_change_pulls(b.ws, 25).await.unwrap(),
        vec![(REPO.to_string(), 7)],
        "closing one workspace's row leaves the other's open"
    );
    assert!(matches!(
        s.settle_change_pull(b.ws, REPO, 7).await.unwrap(),
        ChangeSettlement::Unchanged
    ));
}

async fn agent(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id
}

async fn review(store: &dyn Store, thread: ThreadId, member: MemberId) -> ThreadReview {
    store
        .list_reviews(thread)
        .await
        .unwrap()
        .into_iter()
        .find(|review| review.reviewer_id == member)
        .unwrap()
}

async fn approve(store: &dyn Store, thread: ThreadId, member: MemberId) {
    store
        .submit_review(thread, member, ReviewDecision::Approve, None, None)
        .await
        .unwrap();
}

/// A verdict names the head it was given against (a review child's, its
/// parent change's pull), and a push that moves the head dismisses the
/// approvals given to an earlier one, which then stop counting toward close.
/// An approval of the current head stands, and re-approving names the new
/// head.
async fn a_push_dismisses_approvals_given_to_an_earlier_head(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let room = room(s, "heads").await;
    let thread = task(s, &room).await;
    let (r1, r2, r3) = (
        agent(s, room.ws, "r1").await,
        agent(s, room.ws, "r2").await,
        agent(s, room.ws, "r3").await,
    );
    let child = s
        .create_thread(NewThread {
            channel_id: room.channel,
            parent_thread_id: Some(thread),
            title: Some("review".into()),
            description: None,
            created_by: Some(room.member),
        })
        .await
        .unwrap()
        .id;
    s.set_thread_attributes(
        child,
        TaskAttributes {
            kind: TaskKind::Review,
            ..TaskAttributes::default()
        },
    )
    .await
    .unwrap();
    s.register_change_pull(room.ws, thread, REPO, 7, None)
        .await
        .unwrap();
    s.set_review_requirement(thread, 1).await.unwrap();
    let head = "a".repeat(40);
    s.record_pull_ci(read(room.ws, &head, CiStatus::Success, &[], true))
        .await
        .unwrap();

    // Stamping: the change's verdict and the review child's both name the
    // change's tracked head, in the current row and in the history.
    approve(s, thread, r1).await;
    approve(s, child, r3).await;
    assert_eq!(
        review(s, thread, r1).await.head_sha.as_deref(),
        Some(head.as_str())
    );
    assert_eq!(
        review(s, child, r3).await.head_sha.as_deref(),
        Some(head.as_str())
    );
    let history = s.list_review_history(thread).await.unwrap();
    assert_eq!(history[0].head_sha.as_deref(), Some(head.as_str()));
    assert!(s.review_status(thread).await.unwrap().approvals_met);

    // Re-reading the same head leaves every approval standing.
    let (_, none) = s
        .record_pull_ci_with_events(read(room.ws, &head, CiStatus::Success, &[], true))
        .await
        .unwrap();
    assert!(
        none.is_empty(),
        "a re-read of the same head dismisses nothing"
    );
    assert!(review(s, thread, r1).await.dismissed_at.is_none());
    assert!(review(s, child, r3).await.dismissed_at.is_none());

    // A push moves the head: the earlier approvals, on the change and on its
    // review child, no longer stand and no longer count toward close.
    let pushed = "b".repeat(40);
    let (_, dismissed) = s
        .record_pull_ci_with_events(read(room.ws, &pushed, CiStatus::Pending, &[], false))
        .await
        .unwrap();
    // Each dismissal is an event a waiter can see (#1572).
    let mut said: Vec<(ThreadId, MemberId, String, String)> = dismissed
        .iter()
        .map(|e| match e.opened_event().unwrap() {
            Event::ApprovalDismissed {
                thread_id,
                reviewer_id,
                approved_head,
                head_sha,
                workspace_id,
                ..
            } => {
                assert_eq!(workspace_id, room.ws);
                (thread_id, reviewer_id, approved_head, head_sha)
            }
            other => panic!("not a dismissal: {other:?}"),
        })
        .collect();
    said.sort_by_key(|d| d.0 .0);
    let mut want = vec![
        (thread, r1, head.clone(), pushed.clone()),
        (child, r3, head.clone(), pushed.clone()),
    ];
    want.sort_by_key(|d| d.0 .0);
    assert_eq!(said, want);
    assert!(review(s, thread, r1).await.dismissed_at.is_some());
    assert!(review(s, child, r3).await.dismissed_at.is_some());
    let status = s.review_status(thread).await.unwrap();
    assert_eq!(status.approvals, 0);
    assert!(!status.approvals_met, "a dismissed approval does not count");

    // An approval of the current head survives the next read of it.
    approve(s, thread, r2).await;
    s.record_pull_ci(read(room.ws, &pushed, CiStatus::Success, &[], true))
        .await
        .unwrap();
    let current = review(s, thread, r2).await;
    assert_eq!(current.head_sha.as_deref(), Some(pushed.as_str()));
    assert!(current.dismissed_at.is_none());
    assert!(s.review_status(thread).await.unwrap().approvals_met);

    // Re-approving names the new head and clears the dismissal.
    approve(s, thread, r1).await;
    let renewed = review(s, thread, r1).await;
    assert_eq!(renewed.head_sha.as_deref(), Some(pushed.as_str()));
    assert!(renewed.dismissed_at.is_none());
}

/// Two workspaces tracking the same pull request keep their reviews apart: a
/// CI read in one never dismisses the other's approvals.
async fn a_push_read_in_one_workspace_leaves_anothers_approvals(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let a = room(s, "heads-a").await;
    let b = room(s, "heads-b").await;
    let thread_a = task(s, &a).await;
    let thread_b = task(s, &b).await;
    let reviewer_b = agent(s, b.ws, "reviewer").await;
    for (room, thread) in [(&a, thread_a), (&b, thread_b)] {
        s.register_change_pull(room.ws, thread, REPO, 7, None)
            .await
            .unwrap();
    }
    let head = "c".repeat(40);
    for ws in [a.ws, b.ws] {
        s.record_pull_ci(read(ws, &head, CiStatus::Success, &[], true))
            .await
            .unwrap();
    }
    approve(s, thread_b, reviewer_b).await;

    s.record_pull_ci(read(a.ws, &"d".repeat(40), CiStatus::Pending, &[], false))
        .await
        .unwrap();
    let standing = review(s, thread_b, reviewer_b).await;
    assert_eq!(standing.head_sha.as_deref(), Some(head.as_str()));
    assert!(
        standing.dismissed_at.is_none(),
        "workspace A's read left B alone"
    );
}

/// A read GitHub refused the checks of still moves the head: approvals given
/// to the old head are dismissed, the checks are recorded as unreadable, and
/// no checks_done is said for them (#1623).
async fn a_refused_read_at_a_new_head_dismisses_the_old_heads_approval(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let room = room(s, "refused").await;
    let thread = task(s, &room).await;
    let reviewer = agent(s, room.ws, "reviewer").await;
    s.register_change_pull(room.ws, thread, REPO, 7, None)
        .await
        .unwrap();
    let old = "e".repeat(40);
    s.record_pull_ci(read(room.ws, &old, CiStatus::Success, &[], true))
        .await
        .unwrap();
    approve(s, thread, reviewer).await;
    assert!(review(s, thread, reviewer).await.dismissed_at.is_none());

    let pushed = "f".repeat(40);
    let posted = s
        .record_pull_ci(read(room.ws, &pushed, CiStatus::Unreadable, &[], false))
        .await
        .unwrap();
    assert!(
        review(s, thread, reviewer).await.dismissed_at.is_some(),
        "the approval of the old head no longer stands"
    );
    assert!(!kinds(&posted).contains(&ThreadNoticeKind::ChecksDone));
    let ci = s.get_pull_ci(room.ws, REPO, 7).await.unwrap().unwrap();
    assert_eq!(ci.head_sha, pushed);
    assert_eq!(ci.status, CiStatus::Unreadable);
}

/// The event-surface matrix's evidence that a push publishes each
/// dismissal (#1572); the scenario also runs on both backends below.
#[tokio::test]
async fn a_push_says_each_dismissed_approval_on_sqlite() {
    a_push_dismisses_approvals_given_to_an_earlier_head(sqlite().await).await;
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
    let (container, pool) = postgres_pool().await?;
    Some((container, Arc::new(PostgresStore::for_tests(pool))))
}

async fn postgres_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, sqlx::PgPool)> {
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
    Some((container, pool))
}

/// A reviewer may name the head it read (#1574): the tracked head is
/// stamped as named, and a head the change has moved past is a conflict
/// that records nothing.
async fn a_verdict_naming_a_moved_head_is_refused(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let room = room(s, "named").await;
    let thread = task(s, &room).await;
    let (r1, r2) = (agent(s, room.ws, "r1").await, agent(s, room.ws, "r2").await);
    s.register_change_pull(room.ws, thread, REPO, 7, None)
        .await
        .unwrap();
    let read_head = "a".repeat(40);
    s.record_pull_ci(read(room.ws, &read_head, CiStatus::Success, &[], true))
        .await
        .unwrap();
    let named = read_head.to_ascii_uppercase();
    let stamped = s
        .submit_review_at_head(
            thread,
            r1,
            ReviewDecision::Approve,
            None,
            None,
            Some(&named),
        )
        .await
        .unwrap();
    assert_eq!(stamped.review.head_sha.as_deref(), Some(read_head.as_str()));

    let pushed = "b".repeat(40);
    s.record_pull_ci(read(room.ws, &pushed, CiStatus::Pending, &[], false))
        .await
        .unwrap();
    let refused = s
        .submit_review_at_head(
            thread,
            r2,
            ReviewDecision::Approve,
            None,
            None,
            Some(&read_head),
        )
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "a head the change moved past is a conflict: {refused:?}"
    );
    assert!(
        s.list_reviews(thread)
            .await
            .unwrap()
            .iter()
            .all(|review| review.reviewer_id != r2),
        "a refused verdict records nothing"
    );
}

/// A named head is refused when nothing tracks the change's head, and
/// recording nothing: before a pull request is registered, and after one is
/// registered but not yet read. Omitting the head still works (#1744).
async fn a_named_head_without_a_tracked_pull_is_refused(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let room = room(s, "untracked").await;
    let thread = task(s, &room).await;
    let r1 = agent(s, room.ws, "r1").await;
    let head = "c".repeat(40);
    let refused = s
        .submit_review_at_head(thread, r1, ReviewDecision::Approve, None, None, Some(&head))
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "an unverifiable head is a conflict: {refused:?}"
    );
    s.register_change_pull(room.ws, thread, REPO, 7, None)
        .await
        .unwrap();
    let refused = s
        .submit_review_at_head(thread, r1, ReviewDecision::Approve, None, None, Some(&head))
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "a registered pull with no read head can't verify it: {refused:?}"
    );
    assert!(
        s.list_reviews(thread).await.unwrap().is_empty(),
        "nothing recorded"
    );
    let plain = s
        .submit_review_at_head(thread, r1, ReviewDecision::Approve, None, None, None)
        .await
        .unwrap();
    assert_eq!(plain.review.head_sha, None);
}

/// A push racing an approval never leaves an approval of the old head
/// standing: the verdict either sees the pushed head or is dismissed by the
/// push (#1574).
async fn a_push_during_an_approval_never_leaves_the_old_head_approved(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let room = room(s, "race").await;
    let thread = task(s, &room).await;
    let reviewer = agent(s, room.ws, "r1").await;
    s.register_change_pull(room.ws, thread, REPO, 7, None)
        .await
        .unwrap();
    let ws = room.ws;
    for round in 0..20u8 {
        let old = format!("{round:0>40}");
        let new = format!("{:0>40}", round + 100);
        s.record_pull_ci(read(ws, &old, CiStatus::Success, &[], true))
            .await
            .unwrap();
        let pusher = store.clone();
        let pushed = new.clone();
        let push = tokio::spawn(async move {
            pusher
                .record_pull_ci(read(ws, &pushed, CiStatus::Pending, &[], false))
                .await
        });
        approve(s, thread, reviewer).await;
        push.await.unwrap().unwrap();
        let verdict = review(s, thread, reviewer).await;
        assert!(
            verdict.dismissed_at.is_some() || verdict.head_sha.as_deref() == Some(new.as_str()),
            "round {round}: an approval of {:?} stands after the push to {new}",
            verdict.head_sha
        );
    }
}

/// A CI event marks the open tracked pull requests it names, by number or
/// by the head the sweep last read their CI at, in every workspace that
/// tracks them; a read records its head and start; a closed pull request,
/// another repository or another head is left unmarked.
async fn a_ci_event_marks_open_pulls_by_number_or_last_read_head(store: Arc<dyn Store>) {
    let s = store.as_ref();
    let a = room(s, "ci-a").await;
    let b = room(s, "ci-b").await;
    let (ta, tb) = (task(s, &a).await, task(s, &b).await);
    s.register_change_pull(a.ws, ta, REPO, 7, None)
        .await
        .unwrap();
    s.register_change_pull(b.ws, tb, REPO, 7, None)
        .await
        .unwrap();
    let other = task(s, &a).await;
    s.register_change_pull(a.ws, other, "example/other", 7, None)
        .await
        .unwrap();
    let closed = task(s, &a).await;
    s.register_change_pull(a.ws, closed, REPO, 8, None)
        .await
        .unwrap();

    let fresh = s.change_pull_ci_marks(a.ws, REPO, 7).await.unwrap();
    assert_eq!(
        fresh,
        Some(ChangeCiMarks::default()),
        "nothing read or heard yet"
    );
    assert_eq!(
        s.change_pull_ci_marks(a.ws, REPO, 99).await.unwrap(),
        None,
        "an untracked pull request has no marks"
    );

    // Whole seconds, so both backends give back exactly what was stamped.
    let t0 = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    let head = "ab".repeat(20);
    s.mark_change_pull_ci_read(a.ws, "Example/App", 7, &head.to_ascii_uppercase(), t0)
        .await
        .unwrap();
    s.mark_change_pull_ci_read(a.ws, REPO, 8, &head, t0)
        .await
        .unwrap();
    s.close_change_pull(a.ws, REPO, 8, None, None, t0)
        .await
        .unwrap();
    let read = s
        .change_pull_ci_marks(a.ws, REPO, 7)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.read_at, Some(t0));
    assert_eq!(
        read.read_head.as_deref(),
        Some(head.as_str()),
        "stored lowercase"
    );
    assert_eq!(read.event_at, None);
    assert_eq!(
        s.change_pull_ci_marks(b.ws, REPO, 7)
            .await
            .unwrap()
            .unwrap()
            .read_at,
        None,
        "a read in one workspace is not another's"
    );

    // By number: both workspaces tracking #7 in this repository.
    let t1 = t0 + chrono::Duration::seconds(10);
    assert_eq!(
        s.mark_change_pull_ci_event("EXAMPLE/app", 7, t1)
            .await
            .unwrap(),
        2
    );
    for ws in [a.ws, b.ws] {
        assert_eq!(
            s.change_pull_ci_marks(ws, REPO, 7)
                .await
                .unwrap()
                .unwrap()
                .event_at,
            Some(t1)
        );
    }
    assert_eq!(
        s.change_pull_ci_marks(a.ws, "example/other", 7)
            .await
            .unwrap()
            .unwrap()
            .event_at,
        None,
        "another repository's #7 is not marked"
    );
    assert_eq!(
        s.mark_change_pull_ci_event(REPO, 8, t1).await.unwrap(),
        0,
        "a closed pull request is not read again"
    );
    assert_eq!(s.mark_change_pull_ci_event(REPO, 99, t1).await.unwrap(), 0);

    // By head: only the open pull request whose CI was last read there.
    let t2 = t1 + chrono::Duration::seconds(10);
    assert_eq!(
        s.mark_change_pull_ci_event_at_head(REPO, &head.to_ascii_uppercase(), t2)
            .await
            .unwrap(),
        1,
        "workspace B never read #7 at this head, and #8 is closed"
    );
    assert_eq!(
        s.change_pull_ci_marks(a.ws, REPO, 7)
            .await
            .unwrap()
            .unwrap()
            .event_at,
        Some(t2)
    );
    assert_eq!(
        s.change_pull_ci_marks(b.ws, REPO, 7)
            .await
            .unwrap()
            .unwrap()
            .event_at,
        Some(t1)
    );
    assert_eq!(
        s.mark_change_pull_ci_event_at_head(REPO, &"cd".repeat(20), t2)
            .await
            .unwrap(),
        0,
        "a head nothing was read at marks nothing"
    );
    assert_eq!(
        s.mark_change_pull_ci_event_at_head("example/other", &head, t2)
            .await
            .unwrap(),
        0
    );
}

on_both_backends!(
    a_push_during_an_approval_never_leaves_the_old_head_approved,
    a_ci_event_marks_open_pulls_by_number_or_last_read_head,
    a_verdict_naming_a_moved_head_is_refused,
    a_named_head_without_a_tracked_pull_is_refused,
    a_ci_read_says_each_notice_once_per_head,
    a_review_is_said_once_with_who_how_and_which_head,
    watching_makes_one_thread_and_stays_out_of_the_metrics,
    a_watch_closes_when_its_pull_ends_and_a_change_keeps_its_gates,
    two_workspaces_track_the_same_pull_request_apart,
    a_gate_change_is_said_once,
    a_push_dismisses_approvals_given_to_an_earlier_head,
    a_push_read_in_one_workspace_leaves_anothers_approvals,
    a_refused_read_at_a_new_head_dismisses_the_old_heads_approval,
);
