//! Task-dependency DAG (Cluster 217): edges, dependents, and readiness
//! (all dependencies terminal). Exercised on both backends.

use maidan_fsm::ThreadAction;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
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
    SqliteStore::new(pool)
}

/// Drive a thread all the way to a terminal state (open -> in_review -> closed).
async fn close_thread(
    store: &dyn Store,
    id: maidan_types::ThreadId,
    actor: maidan_types::MemberId,
) {
    store
        .transition_thread(id, actor, ThreadAction::StartReview)
        .await
        .expect("review");
    store
        .transition_thread(id, actor, ThreadAction::Close)
        .await
        .expect("close");
}

async fn run_dag_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "dag".into() })
        .await
        .expect("ws");
    let actor = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "actor".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("actor");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };
    let task = store.create_thread(mk("task")).await.expect("task");
    let dep1 = store.create_thread(mk("dep1")).await.expect("dep1");
    let dep2 = store.create_thread(mk("dep2")).await.expect("dep2");

    // A task with no dependencies is ready.
    assert!(store
        .thread_dependencies_satisfied(task.id)
        .await
        .expect("ready0"));

    // Add two dependencies.
    store
        .add_thread_dependency(task.id, dep1.id)
        .await
        .expect("add dep1");
    store
        .add_thread_dependency(task.id, dep2.id)
        .await
        .expect("add dep2");
    // Idempotent.
    store
        .add_thread_dependency(task.id, dep1.id)
        .await
        .expect("add dep1 again");

    let deps = store
        .list_thread_dependencies(task.id)
        .await
        .expect("list deps");
    assert_eq!(deps.len(), 2, "task depends on two threads");
    let dependents = store
        .list_thread_dependents(dep1.id)
        .await
        .expect("list dependents");
    assert_eq!(dependents.len(), 1, "dep1 blocks one task");
    assert_eq!(dependents[0].thread_id, task.id);

    // Not ready while any dependency is non-terminal (both Open).
    assert!(!store
        .thread_dependencies_satisfied(task.id)
        .await
        .expect("ready1"));

    // Close dep1 (open -> in_review -> closed). Still blocked on dep2.
    store
        .transition_thread(dep1.id, actor.id, ThreadAction::StartReview)
        .await
        .expect("dep1 review");
    store
        .transition_thread(dep1.id, actor.id, ThreadAction::Close)
        .await
        .expect("dep1 close");
    assert!(!store
        .thread_dependencies_satisfied(task.id)
        .await
        .expect("ready2"));

    // Close dep2 too -> all dependencies terminal -> ready.
    store
        .transition_thread(dep2.id, actor.id, ThreadAction::StartReview)
        .await
        .expect("dep2 review");
    store
        .transition_thread(dep2.id, actor.id, ThreadAction::Close)
        .await
        .expect("dep2 close");
    assert!(store
        .thread_dependencies_satisfied(task.id)
        .await
        .expect("ready3"));

    // Self-dependency is rejected.
    assert!(matches!(
        store.add_thread_dependency(task.id, task.id).await,
        Err(maidan_store::StoreError::InvalidInput(_))
    ));

    // Remove is conditional.
    assert!(store
        .remove_thread_dependency(task.id, dep1.id)
        .await
        .expect("remove dep1"));
    assert!(!store
        .remove_thread_dependency(task.id, dep1.id)
        .await
        .expect("remove dep1 again"));
    assert_eq!(
        store
            .list_thread_dependencies(task.id)
            .await
            .expect("list after remove")
            .len(),
        1
    );
}

/// Cluster 218: `claim_next` skips a task whose dependencies aren't all terminal,
/// and picks it up once they are.
async fn run_readiness_claim_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "ready".into(),
        })
        .await
        .expect("ws");
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("agent");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };
    // `blocked` is created first (oldest), but depends on `dep`.
    let blocked = store.create_thread(mk("blocked")).await.expect("blocked");
    let dep = store.create_thread(mk("dep")).await.expect("dep");
    store
        .add_thread_dependency(blocked.id, dep.id)
        .await
        .expect("add dep");

    // claim_next skips the older-but-blocked task and takes the ready one (`dep`).
    let first = store
        .claim_next_thread(channel.id, agent.id, None)
        .await
        .expect("claim1")
        .expect("some ready work");
    assert_eq!(
        first.id, dep.id,
        "the blocked task is skipped for the ready one"
    );

    // Now `dep` is assigned but still Open, so `blocked` is still not ready.
    assert!(store
        .claim_next_thread(channel.id, agent.id, None)
        .await
        .expect("claim2")
        .is_none());

    // Close `dep` -> `blocked` becomes ready and is claimed next.
    store
        .transition_thread(dep.id, agent.id, ThreadAction::StartReview)
        .await
        .expect("dep review");
    store
        .transition_thread(dep.id, agent.id, ThreadAction::Close)
        .await
        .expect("dep close");
    let second = store
        .claim_next_thread(channel.id, agent.id, None)
        .await
        .expect("claim3")
        .expect("blocked is now ready");
    assert_eq!(second.id, blocked.id);
}

/// Cluster 221: a dependency edge that would close a cycle is rejected — direct
/// (A depends on B, then B depends on A) and transitive (A->B->C, then C->A) —
/// while a valid DAG (a diamond) is accepted.
async fn run_cycle_prevention_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "cycles".into(),
        })
        .await
        .expect("ws");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "dag".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };
    let a = store.create_thread(mk("a")).await.expect("a");
    let b = store.create_thread(mk("b")).await.expect("b");
    let c = store.create_thread(mk("c")).await.expect("c");
    let d = store.create_thread(mk("d")).await.expect("d");

    // a depends on b.
    store.add_thread_dependency(a.id, b.id).await.expect("a->b");
    // Direct cycle: b depends on a would loop.
    assert!(
        matches!(
            store.add_thread_dependency(b.id, a.id).await,
            Err(maidan_store::StoreError::InvalidInput(_))
        ),
        "a direct cycle is rejected"
    );

    // b depends on c, so a->b->c.
    store.add_thread_dependency(b.id, c.id).await.expect("b->c");
    // Transitive cycle: c depends on a would close a->b->c->a.
    assert!(
        matches!(
            store.add_thread_dependency(c.id, a.id).await,
            Err(maidan_store::StoreError::InvalidInput(_))
        ),
        "a transitive cycle is rejected"
    );

    // Valid DAG (diamond): a->d and c->d — a shared descendant, no cycle.
    store.add_thread_dependency(a.id, d.id).await.expect("a->d");
    store.add_thread_dependency(c.id, d.id).await.expect("c->d");

    // The rejected edges were never inserted; the accepted ones survived.
    assert_eq!(
        store
            .list_thread_dependencies(a.id)
            .await
            .expect("a deps")
            .len(),
        2,
        "a depends on b and d"
    );
    assert!(store
        .list_thread_dependencies(b.id)
        .await
        .expect("b deps")
        .iter()
        .all(|dep| dep.depends_on_thread_id == c.id));
}

/// Cluster 222: `newly_ready_dependents(dep)` returns the dependents that became
/// ready because `dep` reached a terminal state — empty while another dependency
/// still blocks, the unblocked task once it's the last one, and never a dependent
/// that is itself terminal.
async fn run_ready_dependents_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "ready-ev".into(),
        })
        .await
        .expect("ws");
    let actor = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "actor".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("actor");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "q".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };
    let task = store.create_thread(mk("task")).await.expect("task");
    let dep1 = store.create_thread(mk("dep1")).await.expect("dep1");
    let dep2 = store.create_thread(mk("dep2")).await.expect("dep2");
    store
        .add_thread_dependency(task.id, dep1.id)
        .await
        .expect("task->dep1");
    store
        .add_thread_dependency(task.id, dep2.id)
        .await
        .expect("task->dep2");

    // Closing dep1 leaves task still blocked on dep2 -> not newly ready.
    close_thread(store, dep1.id, actor.id).await;
    assert!(
        store
            .newly_ready_dependents(dep1.id)
            .await
            .expect("after dep1")
            .is_empty(),
        "task still blocked on dep2"
    );

    // Closing dep2 (the last blocker) makes task ready.
    close_thread(store, dep2.id, actor.id).await;
    let ready = store
        .newly_ready_dependents(dep2.id)
        .await
        .expect("after dep2");
    assert_eq!(ready.len(), 1, "task is now ready");
    assert_eq!(ready[0].id, task.id);

    // A dependent that is itself terminal is never reported as ready. Close task,
    // then a fresh dependency of it going terminal must not resurface it.
    close_thread(store, task.id, actor.id).await;
    let late = store.create_thread(mk("late")).await.expect("late");
    store
        .add_thread_dependency(task.id, late.id)
        .await
        .expect("task->late");
    close_thread(store, late.id, actor.id).await;
    assert!(
        store
            .newly_ready_dependents(late.id)
            .await
            .expect("after late")
            .is_empty(),
        "a terminal dependent is never ready"
    );
}

/// Cluster 224: `channel_queue_depth` partitions a channel's open task threads
/// into ready / assigned / blocked, and excludes terminal threads.
async fn run_queue_depth_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "qd".into() })
        .await
        .expect("ws");
    let actor = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "actor".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("actor");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };

    // Empty channel: all zero.
    let d0 = store.channel_queue_depth(channel.id).await.expect("d0");
    assert_eq!(
        d0,
        maidan_types::QueueDepth {
            open: 0,
            ready: 0,
            assigned: 0,
            blocked: 0
        }
    );

    // ready1 (no deps), dep (no deps) → both ready. blocked1 depends on dep.
    let ready1 = store.create_thread(mk("ready1")).await.expect("ready1");
    let dep = store.create_thread(mk("dep")).await.expect("dep");
    let blocked1 = store.create_thread(mk("blocked1")).await.expect("blocked1");
    store
        .add_thread_dependency(blocked1.id, dep.id)
        .await
        .expect("blocked1->dep");
    // assigned1: durable assignment (no lease) → live-assigned.
    let assigned1 = store
        .create_thread(mk("assigned1"))
        .await
        .expect("assigned1");
    store
        .assign_thread(assigned1.id, actor.id)
        .await
        .expect("assign");
    // closed1: terminal → excluded entirely.
    let closed1 = store.create_thread(mk("closed1")).await.expect("closed1");
    close_thread(store, closed1.id, actor.id).await;
    let _ = ready1;

    let d = store.channel_queue_depth(channel.id).await.expect("depth");
    assert_eq!(
        d,
        maidan_types::QueueDepth {
            open: 4,     // ready1 + dep + blocked1 + assigned1 (closed1 excluded)
            ready: 2,    // ready1 + dep
            assigned: 1, // assigned1
            blocked: 1,  // blocked1
        },
        "queue-depth partitions open threads"
    );

    // Closing `dep` unblocks blocked1: blocked -> ready.
    close_thread(store, dep.id, actor.id).await;
    let d2 = store.channel_queue_depth(channel.id).await.expect("depth2");
    assert_eq!(
        d2,
        maidan_types::QueueDepth {
            open: 3,     // dep is now terminal
            ready: 2,    // ready1 + blocked1 (now unblocked)
            assigned: 1, // assigned1
            blocked: 0,
        },
        "closing the dependency moves blocked1 to ready"
    );
}

#[tokio::test]
async fn thread_dependency_dag_edges_and_readiness_sqlite() {
    let store = sqlite().await;
    run_dag_suite(&store).await;
    run_readiness_claim_suite(&store).await;
    run_cycle_prevention_suite(&store).await;
    run_ready_dependents_suite(&store).await;
    run_queue_depth_suite(&store).await;
}

#[tokio::test]
async fn thread_dependency_dag_edges_and_readiness_postgres() {
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
            eprintln!("skipping: docker unavailable ({err})");
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
    let store = PostgresStore::new(pool);
    run_dag_suite(&store).await;
    run_readiness_claim_suite(&store).await;
    run_cycle_prevention_suite(&store).await;
    run_ready_dependents_suite(&store).await;
    run_queue_depth_suite(&store).await;
}
