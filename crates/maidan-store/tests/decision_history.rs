//! Decision history: every review verdict and every land-gate verdict is
//! appended, so a re-submission, a dismissal or clearing the gate leaves the
//! earlier verdicts readable. The migration backfills history from the rows
//! that existed before it. Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    LandColor, LandGateStatus, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    ReviewDecision, ThreadId, LAND_GATE_SKILL,
};
use sqlx::sqlite::SqlitePoolOptions;

const SQLITE_0121: &str = include_str!("../../../migrations/sqlite/0121_decision_history.sql");
const POSTGRES_0121: &str = include_str!("../../../migrations/postgres/0121_decision_history.sql");
const DROP_HISTORY: &str = "DROP TABLE maidan_thread_review_verdicts;
     DROP TABLE maidan_thread_land_gate_verdicts;";

async fn sqlite() -> (SqliteStore, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    (SqliteStore::for_tests(pool.clone()), pool)
}

/// Returns the thread the suite wrote to, for the backfill check.
async fn run_suite(store: &dyn Store) -> ThreadId {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let mk = |handle: &'static str| {
        let ws = ws.id;
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect(handle)
        }
    };
    let owner = mk("owner").await;
    let r1 = mk("r1").await;
    let r2 = mk("r2").await;
    let checker = mk("checker").await;
    store
        .add_member_skill(checker.id, LAND_GATE_SKILL)
        .await
        .expect("skill");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
        })
        .await
        .expect("thread");
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .expect("owner");

    assert!(store
        .list_review_history(thread.id)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .list_land_gate_history(thread.id)
        .await
        .unwrap()
        .is_empty());

    // r1 approves, then changes its mind; r2 requests changes.
    store
        .submit_review(thread.id, r1.id, ReviewDecision::Approve, Some("lgtm"))
        .await
        .expect("r1 approve");
    store
        .submit_review(
            thread.id,
            r1.id,
            ReviewDecision::RequestChanges,
            Some("missed a case"),
        )
        .await
        .expect("r1 flip");
    store
        .submit_review(thread.id, r2.id, ReviewDecision::RequestChanges, None)
        .await
        .expect("r2");

    // The current row for r1 holds only the latest verdict...
    let current = store.list_reviews(thread.id).await.unwrap();
    assert_eq!(current.len(), 2);
    let r1_now = current.iter().find(|r| r.reviewer_id == r1.id).unwrap();
    assert_eq!(r1_now.decision, ReviewDecision::RequestChanges);

    // ...and the history keeps all three, oldest first.
    let history = store.list_review_history(thread.id).await.unwrap();
    let got: Vec<_> = history
        .iter()
        .map(|v| (v.reviewer_id, v.decision, v.note.clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            (r1.id, ReviewDecision::Approve, Some("lgtm".to_string())),
            (
                r1.id,
                ReviewDecision::RequestChanges,
                Some("missed a case".to_string())
            ),
            (r2.id, ReviewDecision::RequestChanges, None),
        ]
    );
    assert!(history.windows(2).all(|w| w[0].id < w[1].id));
    assert!(history.iter().all(|v| v.thread_id == thread.id));
    assert!(history.iter().all(|v| v.actor_id.is_none()));

    // Land gate: amber, then fail, then green. The pointer keeps the last.
    store.require_land_gate(thread.id).await.expect("require");
    // Arming the gate is not a verdict.
    assert!(store
        .list_land_gate_history(thread.id)
        .await
        .unwrap()
        .is_empty());
    store
        .set_land_gate_pointer(
            thread.id,
            checker.id,
            LandGateStatus::Pass,
            Some("abc123"),
            Some(LandColor::Amber),
        )
        .await
        .expect("amber");
    store
        .set_land_gate_pointer(thread.id, checker.id, LandGateStatus::Fail, None, None)
        .await
        .expect("fail");
    store
        .set_land_gate_pointer(
            thread.id,
            checker.id,
            LandGateStatus::Pass,
            Some("def456"),
            None,
        )
        .await
        .expect("green");
    let standing = store.get_land_gate_standing(thread.id).await.unwrap();
    assert_eq!(
        standing.pointer.as_ref().unwrap().artifact_sha.as_deref(),
        Some("def456")
    );

    let gate = store.list_land_gate_history(thread.id).await.unwrap();
    let got: Vec<_> = gate
        .iter()
        .map(|v| (v.status, v.land, v.artifact_sha.clone(), v.recorded_by))
        .collect();
    assert_eq!(
        got,
        vec![
            (
                LandGateStatus::Pass,
                LandColor::Amber,
                Some("abc123".to_string()),
                checker.id
            ),
            (LandGateStatus::Fail, LandColor::Red, None, checker.id),
            (
                LandGateStatus::Pass,
                LandColor::Green,
                Some("def456".to_string()),
                checker.id
            ),
        ]
    );
    assert!(gate.windows(2).all(|w| w[0].id < w[1].id));

    // Clearing the gate removes the pointer, not the history.
    assert!(store.clear_land_gate(thread.id).await.unwrap());
    assert_eq!(
        store.list_land_gate_history(thread.id).await.unwrap().len(),
        3
    );
    assert_eq!(store.list_review_history(thread.id).await.unwrap().len(), 3);

    // Put a current pointer back so the backfill check has one to copy.
    store.require_land_gate(thread.id).await.expect("re-arm");
    store
        .set_land_gate_pointer(thread.id, checker.id, LandGateStatus::Pass, None, None)
        .await
        .expect("re-pass");
    thread.id
}

/// After re-running the migration on an empty history, the history holds one
/// verdict per current review row and one for the current gate pointer.
async fn assert_backfilled(store: &dyn Store, thread_id: ThreadId) {
    let current = store.list_reviews(thread_id).await.unwrap();
    let history = store.list_review_history(thread_id).await.unwrap();
    assert_eq!(history.len(), current.len());
    for row in &current {
        assert!(
            history
                .iter()
                .any(|v| v.reviewer_id == row.reviewer_id && v.decision == row.decision),
            "backfill is missing {row:?}"
        );
    }
    let gate = store.list_land_gate_history(thread_id).await.unwrap();
    assert_eq!(gate.len(), 1);
    assert_eq!(gate[0].status, LandGateStatus::Pass);
    assert_eq!(gate[0].land, LandColor::Green);
}

#[tokio::test]
async fn decision_history_keeps_every_verdict_sqlite() {
    let (store, pool) = sqlite().await;
    let thread_id = run_suite(&store).await;

    sqlx::raw_sql(DROP_HISTORY)
        .execute(&pool)
        .await
        .expect("drop");
    sqlx::raw_sql(SQLITE_0121)
        .execute(&pool)
        .await
        .expect("re-run 0121");
    assert_backfilled(&store, thread_id).await;
}

#[tokio::test]
async fn decision_history_keeps_every_verdict_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
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
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    let thread_id = run_suite(&store).await;

    sqlx::raw_sql(DROP_HISTORY)
        .execute(&pool)
        .await
        .expect("drop");
    sqlx::raw_sql(POSTGRES_0121)
        .execute(&pool)
        .await
        .expect("re-run 0121");
    assert_backfilled(&store, thread_id).await;
}
