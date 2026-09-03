//! N6 (Cluster 350.6): a required-human approval gate is a *claim gate*. While a
//! gate attached to a thread is `pending`, `claim_next` will not hand that thread
//! to an agent; once a human resolves it, the thread becomes claimable again.
//! Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGateState, MemberKind, NewApprovalGate, NewChannel, NewMember, NewThread, NewWorkspace,
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
        .create_workspace(NewWorkspace { name: "n6".into() })
        .await
        .expect("ws");
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("agent");
    let human = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "human".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("human");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("deploy".into()),
        })
        .await
        .expect("thread");

    // A required-human gate is attached to the thread.
    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: agent.id,
            prompt: "Human sign-off before an agent works this?".into(),
            schema: None,
        })
        .await
        .expect("gate");

    // While the gate is pending, claim_next skips the thread — the human must act.
    assert!(
        store
            .claim_next_thread(channel.id, agent.id, None)
            .await
            .expect("claim while gated")
            .is_none(),
        "a thread with a pending approval gate is not claimable"
    );

    // The human accepts; the block lifts and the thread becomes claimable.
    store
        .resolve_approval_gate(gate.id, human.id, ApprovalGateState::Accepted, None)
        .await
        .expect("resolve")
        .expect("was pending");
    let claimed = store
        .claim_next_thread(channel.id, agent.id, None)
        .await
        .expect("claim after resolve")
        .expect("now claimable");
    assert_eq!(claimed.id, thread.id);
    assert_eq!(claimed.assignee_id, Some(agent.id));
}

async fn run_decline_unblocks(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "n6-decline".into(),
        })
        .await
        .expect("ws");
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("agent");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
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
        })
        .await
        .expect("thread");
    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: agent.id,
            prompt: "?".into(),
            schema: None,
        })
        .await
        .expect("gate");
    // The block is on `pending`; a decision either way lifts it — the resolved
    // outcome is data the claimer reads, not a permanent claim block.
    store
        .resolve_approval_gate(gate.id, agent.id, ApprovalGateState::Declined, None)
        .await
        .expect("resolve")
        .expect("was pending");
    assert!(
        store
            .claim_next_thread(channel.id, agent.id, None)
            .await
            .expect("claim after decline")
            .is_some(),
        "a resolved (declined) gate no longer blocks claiming"
    );
}

#[tokio::test]
async fn pending_gate_blocks_claim_then_resolves_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
    run_decline_unblocks(&store).await;
}

#[tokio::test]
async fn pending_gate_blocks_claim_then_resolves_postgres() {
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
    run_decline_unblocks(&store).await;
}
