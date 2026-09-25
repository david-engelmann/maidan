//! Separation of duties judges the delegate, on both backends.
//!
//! The server suite (`maidan-server/tests/attestation_e2e.rs`) proves the rule
//! end to end on SQLite. The SQL behind it is written twice, once per backend,
//! so this runs the same scenario against each: a delegate that worked a thread
//! as its worker cannot then approve or land it as a reviewer, while an
//! independent delegate for the same reviewer can.

use maidan_fsm::ThreadAction;
use maidan_store::attribution::with_attribution;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGateState, Attribution, DelegationGrantId, LandGateStatus, MemberId, MemberKind,
    NewApprovalGate, NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision,
    LAND_GATE_SKILL,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    SqliteStore::new(pool)
}

fn acting_as(actor: MemberId, subject: MemberId) -> Option<Attribution> {
    Some(Attribution {
        actor_id: actor,
        subject_id: subject,
        grant_id: Some(DelegationGrantId(uuid::Uuid::new_v4())),
    })
}

fn directly(member: MemberId) -> Option<Attribution> {
    Some(Attribution {
        actor_id: member,
        subject_id: member,
        grant_id: None,
    })
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for handle in ["worker", "orchestrator", "outsider", "reviewer", "assigner"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let [worker, orchestrator, outsider, reviewer, assigner] = ids[..] else {
        unreachable!()
    };
    store
        .add_member_skill(reviewer, LAND_GATE_SKILL)
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
        .unwrap();
    let thread = |title: &'static str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };

    // A delegate claiming for the member it acts as has worked the thread; a
    // member assigning a thread to someone else has not.
    let worked = store.create_thread(thread("worked")).await.unwrap();
    with_attribution(
        acting_as(orchestrator, worker),
        store.claim_thread(worked.id, worker),
    )
    .await
    .unwrap();
    let workers = store.list_thread_workers(worked.id).await.unwrap();
    assert!(
        workers.contains(&worker) && workers.contains(&orchestrator),
        "{workers:?}"
    );
    let assigned = store.create_thread(thread("assigned")).await.unwrap();
    with_attribution(directly(assigner), store.assign_thread(assigned.id, worker))
        .await
        .unwrap();
    assert_eq!(
        store.list_thread_workers(assigned.id).await.unwrap(),
        vec![worker],
        "assigning work to someone is not doing it"
    );

    // Reviews: the delegate's borrowed approval of its own work does not count,
    // in the status a client reads or in what a close enforces.
    store.set_review_requirement(worked.id, 1).await.unwrap();
    store
        .transition_thread(worked.id, worker, ThreadAction::StartReview)
        .await
        .unwrap();
    let own = with_attribution(
        acting_as(orchestrator, reviewer),
        store.submit_review(worked.id, reviewer, ReviewDecision::Approve, None),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(own.actor_id, Some(orchestrator));
    assert_eq!(store.review_status(worked.id).await.unwrap().approvals, 0);
    assert!(
        store
            .transition_thread(worked.id, worker, ThreadAction::Close)
            .await
            .is_err(),
        "a close must not count the delegate's approval of its own work"
    );
    let independent = with_attribution(
        acting_as(outsider, reviewer),
        store.submit_review(worked.id, reviewer, ReviewDecision::Approve, None),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(independent.actor_id, Some(outsider));
    assert_eq!(store.review_status(worked.id).await.unwrap().approvals, 1);
    let direct = with_attribution(
        directly(reviewer),
        store.submit_review(worked.id, reviewer, ReviewDecision::Approve, None),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(
        direct.actor_id, None,
        "a reviewer acting for itself records no delegate"
    );

    // The land gate: the same rule on the standing.
    store.require_land_gate(worked.id).await.unwrap();
    let own = with_attribution(
        acting_as(orchestrator, reviewer),
        store.set_land_gate_pointer(worked.id, reviewer, LandGateStatus::Pass, None, None),
    )
    .await
    .unwrap();
    assert!(!own.landable, "{own:?}");
    // The review requirement is met by now, so the land gate is all that stands.
    assert!(
        store
            .transition_thread(worked.id, worker, ThreadAction::Close)
            .await
            .is_err(),
        "a close must not accept the delegate's pass of its own work"
    );
    let independent = with_attribution(
        acting_as(outsider, reviewer),
        store.set_land_gate_pointer(worked.id, reviewer, LandGateStatus::Pass, None, None),
    )
    .await
    .unwrap();
    assert!(independent.landable, "{independent:?}");
    store
        .transition_thread(worked.id, worker, ThreadAction::Close)
        .await
        .unwrap();

    // Approval gates record who actually asked and answered.
    let gate = with_attribution(
        acting_as(orchestrator, worker),
        store.create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: None,
            requested_by: worker,
            prompt: "ship it?".into(),
            schema: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(gate.requested_by, worker);
    assert_eq!(gate.requested_actor_id, Some(orchestrator));
    let resolved = with_attribution(
        acting_as(outsider, reviewer),
        store.resolve_approval_gate(gate.id, reviewer, ApprovalGateState::Accepted, None),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(resolved.resolved_by, Some(reviewer));
    assert_eq!(resolved.resolved_actor_id, Some(outsider));
    let fetched = store.get_approval_gate(gate.id).await.unwrap().unwrap();
    assert_eq!(fetched.requested_actor_id, Some(orchestrator));
    assert_eq!(fetched.resolved_actor_id, Some(outsider));
}

#[tokio::test]
async fn attestation_actors_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn attestation_actors_postgres() {
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
        Ok(container) => container,
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
