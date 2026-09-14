//! LandGate close-gate (Cluster 385.2, Wave 2 #25): a `closed` transition
//! is refused until a qualifying green pass exists when the gate is armed.
//! Amber (flags-then-still-engages) is not a land. No row is additive.
//! Both backends.

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    LandColor, LandGateStatus, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    LAND_GATE_SKILL,
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
    let checker = mk("land_gate").await;
    store
        .add_member_skill(checker.id, LAND_GATE_SKILL)
        .await
        .unwrap();
    store
        .add_member_skill(owner.id, LAND_GATE_SKILL)
        .await
        .unwrap();
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

    // Additive: no LandGate row → close as before.
    let t = mk_thread().await;
    store.set_thread_owner(t.id, Some(owner.id)).await.unwrap();
    store.assign_thread(t.id, assignee.id).await.unwrap();
    store
        .transition_thread(t.id, owner.id, ThreadAction::StartReview)
        .await
        .unwrap();
    let closed = store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await
        .expect("no pointer → close");
    assert_eq!(closed.to_state.as_str(), "closed");

    // Require (pending) blocks close.
    let t = mk_thread().await;
    store.set_thread_owner(t.id, Some(owner.id)).await.unwrap();
    store.assign_thread(t.id, assignee.id).await.unwrap();
    store.require_land_gate(t.id).await.unwrap();
    store
        .transition_thread(t.id, owner.id, ThreadAction::StartReview)
        .await
        .unwrap();
    let blocked = store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(blocked, Err(StoreError::Conflict(ref m)) if m.contains("land gate")),
        "pending require must block close, got {blocked:?}"
    );

    // Amber is not a land.
    store
        .set_land_gate_pointer(
            t.id,
            checker.id,
            LandGateStatus::Pass,
            None,
            Some(LandColor::Amber),
        )
        .await
        .unwrap();
    let amber = store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(amber, Err(StoreError::Conflict(ref m)) if m.contains("amber")),
        "amber must not land, got {amber:?}"
    );

    // Fail is red.
    store
        .set_land_gate_pointer(t.id, checker.id, LandGateStatus::Fail, None, None)
        .await
        .unwrap();
    let fail = store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(fail, Err(StoreError::Conflict(ref m)) if m.contains("red") || m.contains("land gate")),
        "fail must not land, got {fail:?}"
    );

    // Implementer (owner) pass does not land.
    store
        .set_land_gate_pointer(t.id, owner.id, LandGateStatus::Pass, None, None)
        .await
        .unwrap();
    let self_pass = store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(self_pass, Err(StoreError::Conflict(ref m)) if m.contains("land gate")),
        "implementer pass must not land, got {self_pass:?}"
    );

    // Qualifying green pass lands.
    store
        .set_land_gate_pointer(t.id, checker.id, LandGateStatus::Pass, None, None)
        .await
        .unwrap();
    let closed = store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await
        .expect("qualifying green pass → close");
    assert_eq!(closed.to_state.as_str(), "closed");
}

#[tokio::test]
async fn land_gate_gate_blocks_close_until_green_pass_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn land_gate_gate_blocks_close_until_green_pass_postgres() {
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
