//! CI on tracked pull requests on both backends: a read of a pull request's
//! checks is recorded on its thread and owes each notice once per head, a
//! watched pull request gets a thread of its own and stays out of the swarm
//! metrics, and nothing crosses a workspace. Postgres runs under
//! testcontainers (skipped without Docker).

use std::sync::Arc;
use std::time::Duration;

use maidan_store::pull_watch::{watch_pull, WatchRequest};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
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
    }
}

/// What holds a pull request back is said when it changes, and only then:
/// blocked with the reasons, clear when none are left, nothing for a read
/// that could not say (which keeps the last answer).
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
    assert_eq!(
        gate_kinds(s.record_pull_ci(gated(Some(&[]))).await.unwrap()),
        vec![(ThreadNoticeKind::GateClear, Vec::new())]
    );
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

/// Another workspace can neither watch a pull request this one tracks nor
/// record checks on it, and nothing is made trying; a channel it names from
/// this workspace is not found.
async fn another_workspaces_pull_is_neither_watched_nor_recorded(store: Arc<dyn Store>) {
    let a = room(store.as_ref(), "a").await;
    let b = room(store.as_ref(), "b").await;
    let s = store.as_ref();
    let thread = task(s, &a).await;
    s.register_change_pull(a.ws, thread, REPO, 7, None)
        .await
        .unwrap();

    let refused = watch_pull(
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
    .unwrap_err();
    assert!(matches!(refused, StoreError::Conflict(_)), "{refused:?}");
    assert!(
        s.list_channels(b.ws)
            .await
            .unwrap()
            .iter()
            .all(|c| c.name != DEFAULT_WATCH_CHANNEL),
        "nothing is made for a refused watch"
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

    assert!(s
        .record_pull_ci(read(b.ws, "aaa1", CiStatus::Failure, &["unit tests"], true))
        .await
        .unwrap()
        .is_empty());
    assert!(s.get_pull_ci(b.ws, REPO, 7).await.unwrap().is_none());
    assert!(s.get_pull_ci(a.ws, REPO, 7).await.unwrap().is_none());
    assert!(s.list_thread_notices(thread, 10).await.unwrap().is_empty());
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
    a_ci_read_says_each_notice_once_per_head,
    watching_makes_one_thread_and_stays_out_of_the_metrics,
    another_workspaces_pull_is_neither_watched_nor_recorded,
    a_gate_change_is_said_once,
);
