//! Lagged resume drains the durable log after a watermark instead of dropping
//! events.

use maidan_store::{prelude::*, resume_from_log, run_sqlite_migrations};
use maidan_types::{Event, MemberKind, NewMember, NewWorkspace};
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

#[tokio::test]
async fn resume_from_log_replays_events_after_the_watermark() {
    let store = sqlite().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "lag-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let stored = store
            .append_event(&Event::MemberJoined {
                occurred_at: chrono::Utc::now(),
                workspace_id: ws.id,
                member: member.clone(),
            })
            .await
            .unwrap();
        ids.push(stored.id);
    }

    let seen = std::sync::Mutex::new(Vec::new());
    let hw = resume_from_log(&store, ids[0], |page| {
        let seen = &seen;
        async move {
            seen.lock().unwrap().extend(page.into_iter().map(|r| r.id));
        }
    })
    .await
    .unwrap();

    assert_eq!(*seen.lock().unwrap(), vec![ids[1], ids[2]]);
    assert_eq!(hw, ids[2]);

    let empty = std::sync::Mutex::new(Vec::new());
    let hw2 = resume_from_log(&store, ids[2], |page| {
        let empty = &empty;
        async move {
            empty.lock().unwrap().extend(page.into_iter().map(|r| r.id));
        }
    })
    .await
    .unwrap();
    assert!(empty.lock().unwrap().is_empty());
    assert_eq!(hw2, ids[2]);
}
