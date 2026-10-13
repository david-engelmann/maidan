//! `list_events_matching` on both stores: the SQL-filtered event read
//! behind filtered projector shapes (the notices query), plus the plan
//! shape that makes it worth having — the 0164 index, used.
//!
//! The semantic suite's oracle is the old behaviour itself: page
//! `list_events_after` and drop non-matching rows with
//! `ProjectorShape::matches_stored`. The filtered read must return the
//! same rows in the same order for every shape.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, EventKind, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    ProjectorShape, ThreadId,
};
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

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "matching".into(),
        })
        .await
        .expect("ws");
    let author = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "author".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let ch1 = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "busy".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch1");
    let ch2 = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "notices".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch2");
    let (th1, _) = store
        .create_thread_with_event(NewThread {
            channel_id: ch1.id,
            parent_thread_id: None,
            title: Some("t1".into()),
            description: None,
        })
        .await
        .expect("th1");
    let (th2, _) = store
        .create_thread_with_event(NewThread {
            channel_id: ch2.id,
            parent_thread_id: None,
            title: Some("t2".into()),
            description: None,
        })
        .await
        .expect("th2");
    for (thread, body) in [
        (th1.id, "one"),
        (th1.id, "two"),
        (th2.id, "three"),
        (th1.id, "four"),
        (th2.id, "five"),
    ] {
        store
            .post_message_with_event(
                maidan_types::NewMessage {
                    thread_id: thread,
                    author_id: author.id,
                    body: body.into(),
                    metadata: serde_json::json!({}),
                    content: None,
                },
                None,
            )
            .await
            .expect("post");
    }

    // The oracle: the unfiltered read, filtered in memory exactly the way
    // the delivery loop used to filter it.
    let all = store
        .list_events_after(ws.id, 0, 10_000)
        .await
        .expect("oracle read");
    // Two thread creations and five posts (the plain create_workspace /
    // create_member calls append no events; only the _with_event variants
    // do, and the channels come from those).
    assert!(
        all.len() == 7,
        "the seed produced the thread and post events: {:?}",
        all.iter()
            .map(|e| (e.kind, e.channel_id))
            .collect::<Vec<_>>()
    );

    let shapes: Vec<(Option<ChannelId>, Option<ThreadId>, Vec<EventKind>)> = vec![
        (Some(ch1.id), None, vec![]),
        (Some(ch2.id), None, vec![EventKind::MessagePosted]),
        (None, None, vec![EventKind::MessagePosted]),
        (None, Some(th1.id), vec![]),
        (
            Some(ch1.id),
            None,
            vec![EventKind::MessagePosted, EventKind::ThreadCreated],
        ),
        (Some(ch2.id), Some(th2.id), vec![EventKind::MessagePosted]),
        (Some(ch2.id), None, vec![EventKind::VoteCast]),
        (None, None, vec![]),
    ];
    for (channel_id, thread_id, kinds) in &shapes {
        let shape = ProjectorShape {
            workspace_id: ws.id,
            channel_id: *channel_id,
            thread_id: *thread_id,
            types: kinds.clone(),
        };
        let expected: Vec<i64> = all
            .iter()
            .filter(|e| shape.matches_stored(e))
            .map(|e| e.id)
            .collect();
        let got = store
            .list_events_matching(ws.id, *channel_id, *thread_id, kinds, 0, 10_000)
            .await
            .expect("matching read");
        assert_eq!(
            got.iter().map(|e| e.id).collect::<Vec<_>>(),
            expected,
            "shape channel={channel_id:?} thread={thread_id:?} kinds={kinds:?}"
        );

        // The keyset: paging through the matches two at a time reaches the
        // same rows, in order, with no repeats.
        if expected.len() > 2 {
            let mut seen: Vec<i64> = Vec::new();
            let mut after = 0;
            loop {
                let page = store
                    .list_events_matching(ws.id, *channel_id, *thread_id, kinds, after, 2)
                    .await
                    .expect("paged matching read");
                if page.is_empty() {
                    break;
                }
                after = page[page.len() - 1].id;
                seen.extend(page.iter().map(|e| e.id));
                if page.len() < 2 {
                    break;
                }
            }
            assert_eq!(seen, expected, "paged shape {shape:?}");
        }
    }
}

#[tokio::test]
async fn list_events_matching_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn list_events_matching_postgres() {
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

/// The SQLite filtered read plans as a seek on the 0164 index with every
/// predicate an index term — the before-plan used only
/// `idx_events_workspace_id` and post-filtered the whole workspace log.
#[tokio::test]
async fn the_sqlite_filtered_read_uses_the_channel_kind_index() {
    use sqlx::Row as _;
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    run_sqlite_migrations(&pool).await.expect("migrate");
    sqlx::query(
        "INSERT INTO maidan_events (kind, workspace_id, channel_id, thread_id, payload, occurred_at)
         WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 6000)
         SELECT CASE WHEN n % 60 = 0 THEN 'message_posted' ELSE 'thread_state_changed' END,
                '00000000-0000-0000-0000-0000000000a1',
                CASE WHEN n % 60 = 0 THEN '00000000-0000-0000-0000-0000000000a2'
                     ELSE '00000000-0000-0000-0000-0000000000a3' END,
                NULL, '{}', '2026-01-01T00:00:00Z' FROM g",
    )
    .execute(&pool)
    .await
    .expect("seed");
    sqlx::query("ANALYZE")
        .execute(&pool)
        .await
        .expect("analyze");
    let rows = sqlx::query(
        "EXPLAIN QUERY PLAN
         SELECT e.id FROM maidan_events e
         LEFT JOIN maidan_content_keys k ON k.id = e.content_key_id
         WHERE e.workspace_id = '00000000-0000-0000-0000-0000000000a1'
           AND e.id > 0
           AND e.channel_id = '00000000-0000-0000-0000-0000000000a2'
           AND e.kind IN ('message_posted')
         ORDER BY e.id ASC LIMIT 500",
    )
    .fetch_all(&pool)
    .await
    .expect("explain");
    let plan: Vec<String> = rows.iter().map(|r| r.get::<String, _>("detail")).collect();
    assert!(
        plan.iter()
            .any(|line| line.contains("idx_events_workspace_channel_kind")),
        "the filtered read seeks on the 0164 index: {plan:?}"
    );
}

/// The Postgres twin of the plan assertion; runs in CI under testcontainers.
#[tokio::test]
async fn the_postgres_filtered_read_uses_the_channel_kind_index() {
    use maidan_store::run_postgres_migrations;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::Row as _;
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
    sqlx::query(
        "INSERT INTO maidan_events (kind, workspace_id, channel_id, thread_id, payload, occurred_at)
         SELECT CASE WHEN g % 300 = 0 THEN 'message_posted' ELSE 'thread_state_changed' END,
                '00000000-0000-0000-0000-0000000000a1'::uuid,
                CASE WHEN g % 300 = 0 THEN '00000000-0000-0000-0000-0000000000a2'::uuid
                     ELSE '00000000-0000-0000-0000-0000000000a3'::uuid END,
                NULL, '{}'::jsonb, now()
         FROM generate_series(1, 30000) AS g",
    )
    .execute(&pool)
    .await
    .expect("seed");
    sqlx::query("ANALYZE maidan_events")
        .execute(&pool)
        .await
        .expect("analyze");
    let rows = sqlx::query(
        "EXPLAIN
         SELECT e.id FROM maidan_events e
         LEFT JOIN maidan_content_keys k ON k.id = e.content_key_id
         WHERE e.workspace_id = '00000000-0000-0000-0000-0000000000a1'
           AND e.id > 0
           AND e.channel_id = '00000000-0000-0000-0000-0000000000a2'
           AND e.kind = ANY(ARRAY['message_posted'])
         ORDER BY e.id ASC LIMIT 500",
    )
    .fetch_all(&pool)
    .await
    .expect("explain");
    let plan: Vec<String> = rows
        .iter()
        .map(|r| r.get::<String, _>("QUERY PLAN"))
        .collect();
    // The event log is partitioned (#1359), so each partition carries its own
    // copy of the 0164 index under a generated name. What matters is that the
    // scan seeks on channel and kind rather than filtering the workspace.
    let seeks = plan.windows(2).any(|pair| {
        pair[0].contains("Index")
            && pair[0].contains("Scan")
            && pair[1].contains("Index Cond")
            && pair[1].contains("channel_id =")
            && pair[1].contains("kind =")
    });
    assert!(
        seeks,
        "the filtered read seeks on the 0164 (workspace, channel, kind, id) index: {plan:?}"
    );
}
