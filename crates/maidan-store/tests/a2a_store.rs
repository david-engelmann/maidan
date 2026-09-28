//! A2A task, context and push config persistence on both stores.

use chrono::{Duration, TimeZone, Utc};
use maidan_store::{
    prelude::*, run_sqlite_migrations, A2aPushConfigRow, A2aTaskQuery, A2aTaskWrite,
};
use maidan_types::{NewChannel, NewThread, NewWorkspace, WorkspaceId};
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
    SqliteStore::for_tests(pool)
}

fn write<'a>(
    workspace_id: WorkspaceId,
    task_id: &'a str,
    context_id: &'a str,
    state: &'a str,
    minute: i64,
) -> A2aTaskWrite<'a> {
    let status_at =
        Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap() + Duration::minutes(minute);
    A2aTaskWrite {
        workspace_id,
        task_id,
        context_id: Some(context_id),
        state,
        status_at,
        task_json: serde_json::json!({"id": task_id, "contextId": context_id, "status": {"state": state}}),
    }
}

fn ids(rows: &[maidan_store::A2aTaskRow]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "a2a".into() })
        .await
        .expect("workspace");
    let other = store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .expect("other workspace");

    // Tasks: minute order t0 < t1 < t2 < t3; t3 lives in another workspace.
    for (id, ctx, state, minute) in [
        ("t0", "c1", "TASK_STATE_COMPLETED", 0),
        ("t1", "c2", "TASK_STATE_CANCELED", 1),
        ("t2", "c1", "TASK_STATE_COMPLETED", 2),
    ] {
        store
            .upsert_a2a_task(write(ws.id, id, ctx, state, minute))
            .await
            .expect("upsert");
    }
    store
        .upsert_a2a_task(write(other.id, "t3", "c1", "TASK_STATE_COMPLETED", 3))
        .await
        .expect("upsert other");

    let row = store.get_a2a_task("t1").await.expect("get").expect("row");
    assert_eq!(row.workspace_id, ws.id);
    assert_eq!(row.task_json["contextId"], "c2");
    assert_eq!(
        row.updated_at,
        Utc.with_ymd_and_hms(2026, 9, 28, 12, 1, 0).unwrap(),
        "updated_at is the status timestamp"
    );
    assert!(store.get_a2a_task("missing").await.expect("get").is_none());

    let all = |limit| A2aTaskQuery {
        limit,
        ..Default::default()
    };
    // Newest status first, workspace-scoped.
    let listed = store.list_a2a_tasks(ws.id, all(10)).await.expect("list");
    assert_eq!(ids(&listed), vec!["t2", "t1", "t0"]);

    // Keyset paging resumes strictly after the cursor.
    let first = store.list_a2a_tasks(ws.id, all(2)).await.expect("page 1");
    assert_eq!(ids(&first), vec!["t2", "t1"]);
    let last = &first[1];
    let second = store
        .list_a2a_tasks(
            ws.id,
            A2aTaskQuery {
                before: Some((last.updated_at, last.id.as_str())),
                limit: 2,
                ..Default::default()
            },
        )
        .await
        .expect("page 2");
    assert_eq!(ids(&second), vec!["t0"]);

    // Filters.
    let by_context = store
        .list_a2a_tasks(
            ws.id,
            A2aTaskQuery {
                context_id: Some("c1"),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .expect("by context");
    assert_eq!(ids(&by_context), vec!["t2", "t0"]);
    let by_state = store
        .list_a2a_tasks(
            ws.id,
            A2aTaskQuery {
                state: Some("TASK_STATE_CANCELED"),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .expect("by state");
    assert_eq!(ids(&by_state), vec!["t1"]);
    let after = store
        .list_a2a_tasks(
            ws.id,
            A2aTaskQuery {
                updated_since: Some(Utc.with_ymd_and_hms(2026, 9, 28, 12, 1, 0).unwrap()),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .expect("after");
    assert_eq!(ids(&after), vec!["t2", "t1"], "at or after");

    // Counts group by context and honour the filters.
    let mut counts = store
        .count_a2a_tasks_by_context(ws.id, all(0))
        .await
        .expect("count");
    counts.sort();
    assert_eq!(
        counts,
        vec![(Some("c1".to_string()), 2), (Some("c2".to_string()), 1)]
    );
    let completed = store
        .count_a2a_tasks_by_context(
            ws.id,
            A2aTaskQuery {
                state: Some("TASK_STATE_COMPLETED"),
                updated_since: Some(Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap()),
                ..Default::default()
            },
        )
        .await
        .expect("count filtered");
    assert_eq!(completed, vec![(Some("c1".to_string()), 2)]);

    // A status change moves the task to the head of the list.
    store
        .upsert_a2a_task(write(ws.id, "t0", "c1", "TASK_STATE_CANCELED", 9))
        .await
        .expect("re-upsert");
    let listed = store.list_a2a_tasks(ws.id, all(10)).await.expect("list");
    assert_eq!(ids(&listed), vec!["t0", "t2", "t1"]);

    // Contexts bind once; a second bind returns the first thread.
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "ctx".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let mut threads = Vec::new();
    for _ in 0..2 {
        threads.push(
            store
                .create_thread(NewThread {
                    channel_id: channel.id,
                    parent_thread_id: None,
                    title: None,
                })
                .await
                .expect("thread")
                .id,
        );
    }
    assert_eq!(
        store
            .get_a2a_context_thread(ws.id, "conv-1")
            .await
            .expect("get"),
        None
    );
    let bound = store
        .bind_a2a_context(ws.id, "conv-1", threads[0])
        .await
        .expect("bind");
    assert_eq!(bound, threads[0]);
    let again = store
        .bind_a2a_context(ws.id, "conv-1", threads[1])
        .await
        .expect("rebind");
    assert_eq!(again, threads[0], "the first binding wins");
    assert_eq!(
        store
            .get_a2a_context_thread(ws.id, "conv-1")
            .await
            .expect("get"),
        Some(threads[0])
    );
    assert_eq!(
        store
            .get_a2a_context_thread(other.id, "conv-1")
            .await
            .expect("get other"),
        None,
        "contexts are per workspace"
    );

    // Push configs.
    let config = |task: &str, id: &str, url: &str| A2aPushConfigRow {
        task_id: task.into(),
        config_id: id.into(),
        url: url.into(),
        token_ciphertext: None,
        auth_scheme: None,
        auth_credentials_ciphertext: None,
    };
    store
        .upsert_a2a_task_push_config(&config("t0", "a", "https://a.example"))
        .await
        .expect("create a");
    store
        .upsert_a2a_task_push_config(&A2aPushConfigRow {
            token_ciphertext: Some("sealed-token".into()),
            auth_scheme: Some("Bearer".into()),
            auth_credentials_ciphertext: Some("sealed-credentials".into()),
            ..config("t0", "b", "https://b.example")
        })
        .await
        .expect("create b");
    store
        .upsert_a2a_task_push_config(&config("t1", "c", "https://c.example"))
        .await
        .expect("create c");
    let b = store
        .get_a2a_task_push_config("t0", "b")
        .await
        .expect("get")
        .expect("b");
    assert_eq!(b.auth_scheme.as_deref(), Some("Bearer"));
    assert_eq!(b.token_ciphertext.as_deref(), Some("sealed-token"));
    assert_eq!(
        b.auth_credentials_ciphertext.as_deref(),
        Some("sealed-credentials")
    );
    assert!(store
        .get_a2a_task_push_config("t0", "missing")
        .await
        .expect("get")
        .is_none());
    let listed: Vec<String> = store
        .list_a2a_task_push_configs("t0")
        .await
        .expect("list")
        .into_iter()
        .map(|c| c.config_id)
        .collect();
    assert_eq!(listed, vec!["a".to_string(), "b".to_string()]);
    // Upsert replaces in place.
    store
        .upsert_a2a_task_push_config(&config("t0", "b", "https://b2.example"))
        .await
        .expect("replace b");
    let b = store
        .get_a2a_task_push_config("t0", "b")
        .await
        .expect("get")
        .expect("b");
    assert_eq!(b.url, "https://b2.example");
    assert_eq!(b.token_ciphertext, None);
    assert_eq!(
        store
            .list_a2a_task_push_configs("t0")
            .await
            .expect("list")
            .len(),
        2
    );
    // Pages walk the configs in id order, each starting after the last id
    // of the one before; another task's configs never appear.
    for id in ["e", "d"] {
        store
            .upsert_a2a_task_push_config(&config("t0", id, "https://d.example"))
            .await
            .expect("create");
    }
    let mut paged = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = store
            .page_a2a_task_push_configs("t0", after.as_deref(), 2)
            .await
            .expect("page");
        assert!(page.len() <= 2);
        after = page.last().map(|c| c.config_id.clone());
        paged.extend(page.into_iter().map(|c| c.config_id));
        if after.is_none() {
            break;
        }
    }
    assert_eq!(paged, ["a", "b", "d", "e"]);
    assert_eq!(
        store
            .page_a2a_task_push_configs("t0", Some("b"), 10)
            .await
            .expect("page")
            .len(),
        2
    );
    for id in ["d", "e"] {
        store
            .delete_a2a_task_push_config("t0", id)
            .await
            .expect("delete");
    }
    assert!(store
        .delete_a2a_task_push_config("t0", "a")
        .await
        .expect("delete"));
    assert!(!store
        .delete_a2a_task_push_config("t0", "a")
        .await
        .expect("delete again"));
}

#[tokio::test]
async fn a2a_persistence_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a2a_persistence_postgres() {
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
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
    run_suite(&PostgresStore::for_tests(pool)).await;
}
