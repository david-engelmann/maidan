//! Thread lifecycle transitions, against a model (Open Work later-V).
//!
//! The claim lease already has a stateful model test. This is the next
//! machine: random sequences of `start_review`, `request_changes`, `close`
//! and `archive` run against the real store and a small model of each
//! thread's state, including a parent/child pair so the rank ceiling is
//! exercised. After every step each thread's state and transition log match
//! the model. An illegal action, or one that would put a child ahead of its
//! parent, must change nothing.
//!
//! The in-memory engine already has property tests; those do not cover
//! `request_changes` and they never go through the store. This suite does
//! both, on SQLite and Postgres.

use std::time::Duration;

use maidan_fsm::hsm::state_rank;
use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId,
    ThreadState, ThreadTransition,
};
use proptest::prelude::*;
use proptest::test_runner::TestRunner;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::runtime::Runtime;

const THREADS: usize = 3;

/// Thread 0 is the parent of thread 1. Thread 2 has no parent.
const PARENT_OF: [Option<usize>; THREADS] = [None, Some(0), None];

#[derive(Debug, Clone)]
struct Op {
    thread: usize,
    action: ThreadAction,
}

fn op() -> impl Strategy<Value = Op> {
    let action = prop_oneof![
        Just(ThreadAction::StartReview),
        Just(ThreadAction::Close),
        Just(ThreadAction::Archive),
        Just(ThreadAction::RequestChanges),
    ];
    (0..THREADS, action).prop_map(|(thread, action)| Op { thread, action })
}

/// Independent of `maidan_fsm::apply`, so a shared bug in the engine and the
/// store cannot hide. The four legal edges, including the review-to-open
/// return that the in-memory property tests do not drive.
fn spec_next(from: ThreadState, action: ThreadAction) -> Option<ThreadState> {
    match (from, action) {
        (ThreadState::Open, ThreadAction::StartReview) => Some(ThreadState::InReview),
        (ThreadState::InReview, ThreadAction::Close) => Some(ThreadState::Closed),
        (ThreadState::Closed, ThreadAction::Archive) => Some(ThreadState::Archived),
        (ThreadState::InReview, ThreadAction::RequestChanges) => Some(ThreadState::Open),
        _ => None,
    }
}

/// Independent of `parent_allows_transition`: a child may move to `to` only
/// when the parent is not archived and the child's target is not ahead.
fn parent_rank_allows(parent: ThreadState, child_to: ThreadState) -> bool {
    parent != ThreadState::Archived && state_rank(child_to) <= state_rank(parent)
}

struct Model {
    states: [ThreadState; THREADS],
    accepted: [usize; THREADS],
}

impl Model {
    fn new() -> Self {
        Self {
            states: [ThreadState::Open; THREADS],
            accepted: [0; THREADS],
        }
    }

    fn next_state(&self, thread: usize, action: ThreadAction) -> Option<ThreadState> {
        let to = spec_next(self.states[thread], action)?;
        match PARENT_OF[thread] {
            Some(parent) if !parent_rank_allows(self.states[parent], to) => None,
            _ => Some(to),
        }
    }
}

struct World<'a> {
    store: &'a dyn Store,
    threads: [ThreadId; THREADS],
    actor: MemberId,
}

async fn create_thread(
    store: &dyn Store,
    channel_id: ChannelId,
    parent_thread_id: Option<ThreadId>,
    title: &str,
) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id,
            parent_thread_id,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id
}

async fn seed(store: &dyn Store) -> World<'_> {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let actor = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "actor".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap()
        .id;
    let root = create_thread(store, channel, None, "root").await;
    let child = create_thread(store, channel, Some(root), "child").await;
    let free = create_thread(store, channel, None, "free").await;
    World {
        store,
        threads: [root, child, free],
        actor,
    }
}

async fn run_suite(store: &dyn Store, ops: Vec<Op>) {
    let w = seed(store).await;
    let mut model = Model::new();

    for Op { thread, action } in ops {
        let allowed = model.next_state(thread, action);
        let result = w
            .store
            .transition_thread(w.threads[thread], w.actor, action)
            .await;
        match (result, allowed) {
            (Ok(got), Some(to)) => {
                assert_eq!(got.from_state, model.states[thread], "thread {thread} from");
                assert_eq!(got.to_state, to, "thread {thread} to");
                assert_eq!(got.thread.state, to, "thread {thread} row");
                model.states[thread] = to;
                model.accepted[thread] += 1;
            }
            (Err(StoreError::Conflict(_)), None) => {}
            (got, expected) => {
                panic!("thread {thread} {action:?}: store {got:?}, model {expected:?}")
            }
        }

        for (i, id) in w.threads.iter().enumerate() {
            let row = w.store.get_thread(*id).await.unwrap();
            assert_eq!(row.state, model.states[i], "thread {i} state");
            let log = w.store.list_thread_transitions(*id, 50).await.unwrap();
            assert_eq!(log.len(), model.accepted[i], "thread {i} log length");
            if let Some(last) = log.last() {
                assert_eq!(last.from_state, previous_state(&log));
                assert_eq!(last.to_state, model.states[i], "thread {i} last to");
                assert_eq!(last.actor_id, w.actor, "thread {i} actor");
            }
        }
    }
}

/// The state the last transition left from: the one before it landed in,
/// or `Open` (every thread is created open) when it is the first.
fn previous_state(log: &[ThreadTransition]) -> ThreadState {
    match log {
        [.., before, _last] => before.to_state,
        _ => ThreadState::Open,
    }
}

async fn sqlite_store() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    SqliteStore::for_tests(pool)
}

struct PostgresWorld {
    /// Held so the container lives for every proptest case.
    _container: testcontainers::ContainerAsync<Postgres>,
    store: PostgresStore,
}

async fn postgres_world() -> Option<PostgresWorld> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some(PostgresWorld {
        _container: container,
        store: PostgresStore::for_tests(pool),
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn thread_transitions_follow_the_model_sqlite(ops in proptest::collection::vec(op(), 1..40)) {
        Runtime::new().unwrap().block_on(async {
            let store = sqlite_store().await;
            run_suite(&store, ops).await;
        });
    }
}

#[test]
fn thread_transitions_follow_the_model_postgres() {
    let rt = Runtime::new().unwrap();
    let Some(world) = rt.block_on(postgres_world()) else {
        return;
    };
    let mut runner = TestRunner::new(ProptestConfig::with_cases(64));
    runner
        .run(&proptest::collection::vec(op(), 1..40), |ops| {
            rt.block_on(run_suite(&world.store, ops));
            Ok(())
        })
        .expect("thread transitions should follow the model on Postgres");
}
