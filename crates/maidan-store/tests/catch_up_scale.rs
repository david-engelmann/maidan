//! Catch-up stays flat as the event log grows (UI deep dive P2, thread 18).
//! `catch_up_since` pages by keyset (`id > after_lsn`), clamps every page to
//! `CATCH_UP_LIMIT`, and each query it runs seeks an index instead of
//! scanning the event table, so a page costs the same at any log size.

use std::collections::HashSet;

use maidan_store::{catch_up_since, prelude::*, run_sqlite_migrations, CATCH_UP_LIMIT};
use maidan_types::{Event, MemberKind, NewMember, NewWorkspace, WorkspaceId};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    run_sqlite_migrations(&pool).await.expect("migrate");
    pool
}

async fn workspace_with_events(store: &dyn Store, name: &str, extra: usize) -> WorkspaceId {
    let (ws, _) = store
        .create_workspace_with_event(NewWorkspace { name: name.into() })
        .await
        .expect("ws");
    let (member, _) = store
        .create_member_with_event(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-u"),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    for _ in 0..extra {
        store
            .append_event(&Event::MemberJoined {
                occurred_at: chrono::Utc::now(),
                workspace_id: ws.id,
                member: member.clone(),
            })
            .await
            .expect("append");
    }
    ws.id
}

#[tokio::test]
async fn catch_up_pages_are_clamped_keyset_pages_at_any_log_size() {
    let pool = pool().await;
    let store = SqliteStore::for_tests(pool.clone());
    let ws = workspace_with_events(&store, "big", 1_200).await;
    // Another workspace's events interleave in the same table and must not
    // leak into, or slow, this workspace's pages.
    let _noise = workspace_with_events(&store, "noise", 300).await;
    let _ = workspace_with_events(&store, "big-tail", 0).await;

    let mut after = 0;
    let mut seen = HashSet::new();
    let mut pages = 0;
    loop {
        let page = catch_up_since(&store, ws, after, i64::MAX)
            .await
            .expect("page");
        assert!(page.ok(), "{:?}", page.chain);
        assert!(page.events.len() as i64 <= CATCH_UP_LIMIT);
        let mut last = after;
        for e in &page.events {
            assert_eq!(e.workspace_id, Some(ws));
            assert!(e.id > last, "ascending keyset order");
            assert!(seen.insert(e.id), "no event twice");
            last = e.id;
        }
        pages += 1;
        if !page.truncated {
            assert_eq!(last.max(after), page.head_lsn.max(after));
            break;
        }
        assert_eq!(page.events.len() as i64, CATCH_UP_LIMIT);
        after = last;
        assert!(pages < 10, "paging terminates");
    }
    // 1,200 appended plus the workspace and member creation events.
    assert!(seen.len() >= 1_202, "saw {}", seen.len());
    assert_eq!(pages, seen.len().div_ceil(CATCH_UP_LIMIT as usize));
}

#[tokio::test]
async fn every_catch_up_query_seeks_an_index() {
    let pool = pool().await;
    let store = SqliteStore::for_tests(pool.clone());
    let ws = workspace_with_events(&store, "plan", 50).await;
    sqlx::query("ANALYZE")
        .execute(&pool)
        .await
        .expect("analyze");
    let id = ws.0.to_string();
    // The shapes behind list_events_after, the head and floor links, the
    // at-or-before link and the cursor freshness floor.
    for sql in [
        "SELECT e.id FROM maidan_events e WHERE e.workspace_id = ?1 AND e.id > 0 ORDER BY e.id ASC LIMIT 500",
        "SELECT id FROM maidan_events WHERE workspace_id = ?1 ORDER BY id DESC LIMIT 1",
        "SELECT id FROM maidan_events WHERE workspace_id = ?1 ORDER BY id ASC LIMIT 1",
        "SELECT id FROM maidan_events WHERE workspace_id = ?1 AND id <= 40 ORDER BY id DESC LIMIT 1",
        "SELECT MIN(id) FROM maidan_events WHERE workspace_id = ?1",
    ] {
        let rows: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(&format!("EXPLAIN QUERY PLAN {sql}"))
                .bind(&id)
                .fetch_all(&pool)
                .await
                .expect("plan");
        let plan: Vec<&str> = rows.iter().map(|r| r.3.as_str()).collect();
        assert!(
            plan.iter().all(|d| !d.starts_with("SCAN")),
            "{sql} scans: {plan:?}"
        );
        assert!(plan.iter().any(|d| d.contains("INDEX")), "{sql}: {plan:?}");
    }
}
