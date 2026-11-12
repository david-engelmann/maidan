//! A child thread's parent is checked inside the insert's transaction, on both
//! backends. A parent archived while the child is being created refuses the
//! child instead of taking it, and a parent in another channel or workspace
//! reads as missing, the same as an id that does not exist (CONC-002).

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::{ChannelId, NewChannel, NewThread, NewWorkspace, Thread, ThreadId, WorkspaceId};

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

async fn channel(store: &dyn Store, ws: WorkspaceId, name: &str) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: name.into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap()
        .id
}

fn child_of(channel: ChannelId, parent: ThreadId) -> NewThread {
    NewThread {
        channel_id: channel,
        parent_thread_id: Some(parent),
        title: Some("child".into()),
        description: None,
        created_by: None,
    }
}

async fn root(store: &dyn Store, channel: ChannelId) -> Thread {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("parent".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
}

/// Both create paths refuse a parent from another channel of the workspace,
/// or from another workspace, exactly as they refuse an unknown id.
async fn a_parent_elsewhere_reads_as_missing(store: &dyn Store) {
    let ws_a = workspace(store, "a").await;
    let ws_b = workspace(store, "b").await;
    let a1 = channel(store, ws_a, "a1").await;
    let a2 = channel(store, ws_a, "a2").await;
    let b1 = channel(store, ws_b, "b1").await;
    let parent = root(store, a1).await;
    let unknown = ThreadId(uuid::Uuid::now_v7());

    for (ch, id, what) in [
        (a2, parent.id, "another channel"),
        (b1, parent.id, "another workspace"),
        (a1, unknown, "an unknown id"),
    ] {
        let plain = store.create_thread(child_of(ch, id)).await;
        assert!(
            matches!(plain, Err(StoreError::NotFound)),
            "create_thread under {what}: {plain:?}"
        );
        let evented = store.create_thread_with_event(child_of(ch, id)).await;
        assert!(
            matches!(evented, Err(StoreError::NotFound)),
            "create_thread_with_event under {what}: {evented:?}"
        );
    }
    // The parent's own channel still takes the child.
    let child = store.create_thread(child_of(a1, parent.id)).await.unwrap();
    assert_eq!(child.parent_thread_id, Some(parent.id));
}

/// `archive` archives `parent` in a transaction it holds open, and returns a
/// handle that commits it. The child create starts while it is open; the
/// archive commits before the create may finish.
async fn an_archive_during_a_child_create_refuses_the_child<F, C>(
    store: std::sync::Arc<dyn Store>,
    channel: ChannelId,
    parent: ThreadId,
    with_event: bool,
    archive: F,
) where
    F: AsyncFnOnce(ThreadId) -> C,
    C: AsyncFnOnce(),
{
    let commit = archive(parent).await;
    let create = {
        let store = store.clone();
        tokio::spawn(async move {
            if with_event {
                store
                    .create_thread_with_event(child_of(channel, parent))
                    .await
                    .map(|(t, _)| t)
            } else {
                store.create_thread(child_of(channel, parent)).await
            }
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    commit().await;
    let got = create.await.unwrap();
    assert!(
        matches!(&got, Err(StoreError::Conflict(m)) if m.contains("archived")),
        "with_event={with_event}: a child landed under a parent archived mid-create: {got:?}"
    );
    let children: Vec<_> = store
        .list_threads(channel)
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.parent_thread_id == Some(parent))
        .collect();
    assert!(children.is_empty(), "{children:?}");
}

#[tokio::test]
async fn a_parent_elsewhere_reads_as_missing_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    a_parent_elsewhere_reads_as_missing(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn an_archive_during_a_child_create_refuses_the_child_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("p.db").display());
    let pool = maidan_search::sqlite_pool_options_with(10_000)
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: std::sync::Arc<dyn Store> =
        std::sync::Arc::new(SqliteStore::for_tests(pool.clone()));
    let ws = workspace(store.as_ref(), "race").await;
    let ch = channel(store.as_ref(), ws, "work").await;
    for with_event in [false, true] {
        let parent = root(store.as_ref(), ch).await;
        let pool = pool.clone();
        an_archive_during_a_child_create_refuses_the_child(
            store.clone(),
            ch,
            parent.id,
            with_event,
            async move |id| {
                let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
                sqlx::query("UPDATE maidan_threads SET state = 'archived' WHERE id = ?")
                    .bind(id.0)
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                async move || tx.commit().await.unwrap()
            },
        )
        .await;
    }
}

async fn postgres() -> Option<(
    testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    sqlx::PgPool,
)> {
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

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
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some((container, pool))
}

#[tokio::test]
async fn a_parent_elsewhere_reads_as_missing_postgres() {
    let Some((_c, pool)) = postgres().await else {
        return;
    };
    a_parent_elsewhere_reads_as_missing(&PostgresStore::for_tests(pool)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_archive_during_a_child_create_refuses_the_child_postgres() {
    let Some((_c, pool)) = postgres().await else {
        return;
    };
    let store: std::sync::Arc<dyn Store> =
        std::sync::Arc::new(PostgresStore::for_tests(pool.clone()));
    let ws = workspace(store.as_ref(), "race").await;
    let ch = channel(store.as_ref(), ws, "work").await;
    for with_event in [false, true] {
        let parent = root(store.as_ref(), ch).await;
        let pool = pool.clone();
        an_archive_during_a_child_create_refuses_the_child(
            store.clone(),
            ch,
            parent.id,
            with_event,
            async move |id| {
                let mut tx = pool.begin().await.unwrap();
                sqlx::query("UPDATE maidan_threads SET state = 'archived' WHERE id = $1")
                    .bind(id.0)
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                async move || tx.commit().await.unwrap()
            },
        )
        .await;
    }
}
