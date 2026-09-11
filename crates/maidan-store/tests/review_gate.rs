//! Review close-gate (Cluster 375.2, Wave 2 #22): a `closed` transition is
//! refused until `k` qualifying approvals exist AND no `refutes` edge targets the
//! thread. The gate lives in the FSM transition (both backends).

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewReference, NewThread, NewWorkspace, RefSide,
    RelationKind, ReviewDecision,
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
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
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
    let assignee = mk("assignee").await;
    let reviewer = mk("reviewer").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mk_thread = || async {
        store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("t".into()),
            })
            .await
            .expect("thread")
    };

    // --- Part A: the review requirement gate ---
    let t = mk_thread().await;
    store.set_thread_owner(t.id, Some(owner.id)).await.unwrap();
    store.assign_thread(t.id, assignee.id).await.unwrap();
    store.set_review_requirement(t.id, 1).await.unwrap();
    // Open -> InReview (not terminal, ungated).
    store
        .transition_thread(t.id, owner.id, ThreadAction::StartReview)
        .await
        .expect("start review");
    // Close with 0 approvals -> refused (the owner satisfies SoD, but the review
    // gate blocks).
    let blocked = store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(blocked, Err(StoreError::Conflict(ref m)) if m.contains("review requirement")),
        "0 of 1 approvals must block close, got {blocked:?}"
    );
    // An approval from the reviewer (!= owner/assignee) satisfies it.
    store
        .submit_review(t.id, reviewer.id, ReviewDecision::Approve, None)
        .await
        .unwrap();
    let closed = store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await
        .expect("close after approval");
    assert_eq!(closed.to_state.as_str(), "closed");

    // --- Part B: a `refutes` edge blocks close (even with no review requirement) ---
    let t2 = mk_thread().await;
    store.set_thread_owner(t2.id, Some(owner.id)).await.unwrap();
    store.assign_thread(t2.id, assignee.id).await.unwrap();
    store
        .transition_thread(t2.id, owner.id, ThreadAction::StartReview)
        .await
        .unwrap();
    // A message (any UUID) refutes this thread.
    store
        .add_reference(NewReference {
            src_kind: RefSide::Message,
            src_id: uuid::Uuid::new_v4(),
            dst_kind: RefSide::Thread,
            dst_id: t2.id.0,
            relation: RelationKind::Refutes,
        })
        .await
        .expect("refutes ref");
    let refuted = store
        .transition_thread(t2.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(refuted, Err(StoreError::Conflict(ref m)) if m.contains("refutes")),
        "a refutes edge must block close, got {refuted:?}"
    );
}

#[tokio::test]
async fn review_gate_blocks_close_until_approved_and_unrefuted_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn review_gate_blocks_close_until_approved_and_unrefuted_postgres() {
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
            eprintln!("skipping: docker unavailable ({err})");
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
