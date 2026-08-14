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

#[tokio::test]
async fn thread_dependency_dag_edges_and_readiness_sqlite() {
    let store = sqlite().await;
    run_dag_suite(&store).await;
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
}
