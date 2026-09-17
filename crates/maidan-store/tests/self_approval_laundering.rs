//! Releasing a claim no longer launders a self-approval.
//!
//! Both governance gates tested the thread's **live** `assignee_id`. A release
//! sets that to NULL, so the exclusion went vacuous at exactly the moment
//! someone wanted it to: do the work, release, approve your own work as a
//! qualifying third party. The gate still ran — it had nothing left to compare
//! against.
//!
//! Each case below performs the laundering exactly as an attacker would, and
//! asserts the close is still refused. Both backends.

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    LandColor, LandGateStatus, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    ReviewDecision, LAND_GATE_SKILL,
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
        .create_workspace(NewWorkspace {
            name: "launder".into(),
        })
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
    let owner = mk("l-owner").await;
    let worker = mk("l-worker").await;
    let bystander = mk("l-bystander").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "l-c".into(),
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

    // --- The review gate ---
    let t = mk_thread().await;
    store.set_thread_owner(t.id, Some(owner.id)).await.unwrap();
    store.set_review_requirement(t.id, 1).await.unwrap();

    // The worker does the work, then launders: release the claim so the live
    // `assignee_id` no longer names them.
    assert!(
        store.claim_thread(t.id, worker.id).await.unwrap().claimed,
        "the worker held the thread"
    );
    store.unassign_thread(t.id).await.unwrap();
    assert!(
        store.get_thread(t.id).await.unwrap().assignee_id.is_none(),
        "the laundering step really did clear the live column"
    );

    // …and approves their own work.
    store
        .submit_review(t.id, worker.id, ReviewDecision::Approve, None)
        .await
        .unwrap();
    store
        .transition_thread(t.id, owner.id, ThreadAction::StartReview)
        .await
        .expect("start review");
    let refused = store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        refused.is_err(),
        "a self-approval laundered through a claim release must not close the thread"
    );

    // A genuine third party still closes it — the gate must exclude the worker,
    // not everybody.
    store
        .submit_review(t.id, bystander.id, ReviewDecision::Approve, None)
        .await
        .unwrap();
    store
        .transition_thread(t.id, owner.id, ThreadAction::Close)
        .await
        .expect("a third-party approval still closes");

    // --- The land gate ---
    let g = mk_thread().await;
    store.set_thread_owner(g.id, Some(owner.id)).await.unwrap();
    store
        .add_member_skill(worker.id, LAND_GATE_SKILL)
        .await
        .unwrap();
    store
        .add_member_skill(bystander.id, LAND_GATE_SKILL)
        .await
        .unwrap();
    store.require_land_gate(g.id).await.unwrap();

    assert!(store.claim_thread(g.id, worker.id).await.unwrap().claimed);
    store.unassign_thread(g.id).await.unwrap();
    store
        .set_land_gate_pointer(g.id, worker.id, LandGateStatus::Pass, None, None)
        .await
        .unwrap();
    store
        .transition_thread(g.id, owner.id, ThreadAction::StartReview)
        .await
        .expect("start review");
    let refused = store
        .transition_thread(g.id, owner.id, ThreadAction::Close)
        .await;
    assert!(
        refused.is_err(),
        "a green pass from the member who did the work must not land it, \
         even after they release the claim"
    );

    // The standing the room reports agrees with the enforcement — a gate that
    // refuses while showing green would be worse than one that let it through.
    let standing = store.get_land_gate_standing(g.id).await.unwrap();
    assert_ne!(
        standing.land,
        LandColor::Green,
        "the reported standing must not claim green: {standing:?}"
    );

    // A skilled third party lands it.
    store
        .set_land_gate_pointer(g.id, bystander.id, LandGateStatus::Pass, None, None)
        .await
        .unwrap();
    store
        .transition_thread(g.id, owner.id, ThreadAction::Close)
        .await
        .expect("a third-party green pass still lands");
}

#[tokio::test]
async fn a_claim_release_does_not_launder_a_self_approval_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn a_claim_release_does_not_launder_a_self_approval_postgres() {
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
    run_suite(&PostgresStore::new(pool)).await;
}
