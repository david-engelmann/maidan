//! Durable human-approval gates (Cluster 350, the held gate): create a pending
//! gate, list it while outstanding, resolve it to accept/decline/cancel, and
//! prove the resolve is a compare-and-set on `pending` (a double-answer is a
//! no-op). Both backends. No routes/tool yet — the zero-blast-radius foundation.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGateState, MemberKind, NewApprovalGate, NewChannel, NewMember, NewThread, NewWorkspace,
};
use serde_json::json;
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
            name: "gates".into(),
        })
        .await
        .expect("ws");
    let requester = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("requester");
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

    // No gates outstanding at the start.
    assert!(store
        .list_pending_approval_gates(ws.id, 50)
        .await
        .expect("list empty")
        .is_empty());

    // Open a thread-attached gate with a requestedSchema.
    let schema = json!({ "type": "object", "properties": { "ok": { "type": "boolean" } } });
    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: requester.id,
            prompt: "Deploy to prod?".into(),
            schema: Some(schema.clone()),
        })
        .await
        .expect("create");
    assert_eq!(gate.state, ApprovalGateState::Pending);
    assert_eq!(gate.thread_id, Some(thread.id));
    assert_eq!(gate.requested_by, requester.id);
    assert_eq!(gate.schema.as_ref(), Some(&schema));
    assert!(gate.resolved_by.is_none() && gate.resolved_at.is_none());

    // get round-trips it; list_pending surfaces it while outstanding.
    let got = store
        .get_approval_gate(gate.id)
        .await
        .expect("get")
        .expect("some");
    assert_eq!(got.prompt, "Deploy to prod?");
    let pending = store
        .list_pending_approval_gates(ws.id, 50)
        .await
        .expect("list pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, gate.id);

    // A human accepts, with response content.
    let content = json!({ "ok": true, "note": "ship it" });
    let resolved = store
        .resolve_approval_gate(
            gate.id,
            human.id,
            ApprovalGateState::Accepted,
            Some(&content),
        )
        .await
        .expect("resolve")
        .expect("was pending");
    assert_eq!(resolved.state, ApprovalGateState::Accepted);
    assert_eq!(resolved.content.as_ref(), Some(&content));
    assert_eq!(resolved.resolved_by, Some(human.id));
    assert!(resolved.resolved_at.is_some());

    // The resolve is a compare-and-set on `pending`: a second answer is a no-op.
    let again = store
        .resolve_approval_gate(gate.id, human.id, ApprovalGateState::Declined, None)
        .await
        .expect("second resolve ok");
    assert!(again.is_none(), "a resolved gate cannot be re-resolved");
    // ...and the original outcome stands.
    assert_eq!(
        store
            .get_approval_gate(gate.id)
            .await
            .expect("get")
            .expect("some")
            .state,
        ApprovalGateState::Accepted
    );

    // A resolved gate leaves the pending queue.
    assert!(store
        .list_pending_approval_gates(ws.id, 50)
        .await
        .expect("list after resolve")
        .is_empty());

    // Decline and cancel are first-class outcomes; a standalone (no-thread) gate.
    let declined = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: None,
            requested_by: requester.id,
            prompt: "Merge?".into(),
            schema: None,
        })
        .await
        .expect("create declined");
    let d = store
        .resolve_approval_gate(declined.id, human.id, ApprovalGateState::Declined, None)
        .await
        .expect("decline")
        .expect("was pending");
    assert_eq!(d.state, ApprovalGateState::Declined);
    assert!(d.thread_id.is_none());

    let cancelled = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: None,
            requested_by: requester.id,
            prompt: "Roll back?".into(),
            schema: None,
        })
        .await
        .expect("create cancelled");
    let c = store
        .resolve_approval_gate(cancelled.id, human.id, ApprovalGateState::Cancelled, None)
        .await
        .expect("cancel")
        .expect("was pending");
    assert_eq!(c.state, ApprovalGateState::Cancelled);

    // Both resolved → the pending queue is empty again.
    assert!(store
        .list_pending_approval_gates(ws.id, 50)
        .await
        .expect("list final")
        .is_empty());
}

#[tokio::test]
async fn approval_gate_create_list_resolve_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn approval_gate_create_list_resolve_postgres() {
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
}
