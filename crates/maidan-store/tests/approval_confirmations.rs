//! The approval policy and the confirmation links `approval_decide` issues.
//! Both backends: the default policy, setting it, issuing a link, reusing the
//! live one, refusing an expired one, and spending one once.

use chrono::Utc;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGateState, ApprovalRisk, ConfirmOutcome, GateDecisionVia, MemberKind,
    NewApprovalConfirmation, NewApprovalGate, NewMember, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

/// An audit row for a write that must record one.
fn audit<T>(action: &'static str, ws: maidan_types::WorkspaceId) -> maidan_store::AuditFor<T> {
    Box::new(move |_| maidan_types::NewAuditEvent {
        scope: maidan_types::AuditScope::Workspace(ws),
        actor_id: None,
        action: action.into(),
        target_kind: None,
        target_id: None,
        metadata: serde_json::json!({}),
    })
}

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
            name: "confirm".into(),
        })
        .await
        .expect("ws");
    let requester = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "requester".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("requester");
    let approver = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "approver".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("approver");

    // No row means the default: confirm from low, and it is the default.
    let policy = store.get_approval_policy(ws.id).await.expect("policy");
    assert_eq!(policy.confirm_at, ApprovalRisk::Low);
    assert!(policy.is_default);
    assert!(policy.needs_confirmation(ApprovalRisk::Low));
    assert!(policy.needs_confirmation(ApprovalRisk::High));

    let set = store
        .set_approval_policy_audited(
            ws.id,
            Some(ApprovalRisk::High),
            audit("approval_policy.set", ws.id),
        )
        .await
        .expect("set");
    assert_eq!(set.confirm_at, ApprovalRisk::High);
    assert!(!set.is_default);
    assert!(!set.needs_confirmation(ApprovalRisk::Medium));
    assert!(set.needs_confirmation(ApprovalRisk::High));

    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: None,
            requested_by: requester.id,
            prompt: "ship it?".into(),
            schema: None,
            risk: ApprovalRisk::High,
        })
        .await
        .expect("gate");

    let now = Utc::now();
    let nonce = Uuid::now_v7();
    let new = |nonce: Uuid, hash: &str| NewApprovalConfirmation {
        gate_id: gate.id,
        member_id: approver.id,
        workspace_id: ws.id,
        actor_id: None,
        nonce,
        token_hash: hash.to_string(),
        client_name: Some("test-client".into()),
        client_version: Some("1.2.3".into()),
        note: Some("looks right".into()),
        now,
        expires_at: now + chrono::Duration::minutes(10),
    };
    let (issued, fresh) = store
        .issue_approval_confirmation(&new(nonce, "hash-1"), audit("issued", ws.id))
        .await
        .expect("issue");
    assert!(fresh);
    assert_eq!(issued.nonce, nonce);
    assert_eq!(issued.client_name.as_deref(), Some("test-client"));

    // A second issue for the same gate and member returns the live one.
    let (again, fresh) = store
        .issue_approval_confirmation(
            &new(Uuid::now_v7(), "hash-2"),
            Box::new(|_| panic!("a reused link is not audited")),
        )
        .await
        .expect("reissue");
    assert!(!fresh);
    assert_eq!(again.nonce, nonce);

    // The live list names it, and the hash finds it.
    let live = store
        .list_live_approval_confirmations(ws.id, now)
        .await
        .expect("live");
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].gate_id, gate.id);
    let found = store
        .get_approval_confirmation_by_token("hash-1")
        .await
        .expect("lookup")
        .expect("found");
    assert_eq!(found.nonce, nonce);
    assert!(store
        .get_approval_confirmation_by_token("hash-2")
        .await
        .expect("lookup")
        .is_none());

    // An expired link is not live and cannot be spent.
    let expired = store
        .get_approval_confirmation_by_token("hash-1")
        .await
        .expect("lookup")
        .unwrap();
    assert!(!expired.is_live(now + chrono::Duration::hours(1)));
    let outcome = store
        .confirm_approval_gate(
            "hash-1",
            ws.id,
            approver.id,
            now + chrono::Duration::hours(1),
            Box::new(|_| panic!("not audited")),
        )
        .await
        .expect("confirm expired");
    assert!(matches!(outcome, ConfirmOutcome::NotFound));

    // The person it is bound to spends it once, and the gate records the
    // client and the note.
    let via = GateDecisionVia {
        client_name: Some("test-client".into()),
        client_version: Some("1.2.3".into()),
        model_asked: true,
    };
    let outcome = store
        .confirm_approval_gate("hash-1", ws.id, approver.id, now, audit("decided", ws.id))
        .await
        .expect("confirm");
    let ConfirmOutcome::Accepted(resolved) = outcome else {
        panic!("expected accepted, got {outcome:?}");
    };
    let resolved = *resolved;
    assert_eq!(resolved.state, ApprovalGateState::Accepted);
    assert_eq!(resolved.resolved_by, Some(approver.id));
    assert_eq!(resolved.decided_via, Some(via));
    assert_eq!(
        resolved.content,
        Some(serde_json::json!({ "note": "looks right" }))
    );

    // Spent: not found again, and no longer live.
    let again = store
        .confirm_approval_gate(
            "hash-1",
            ws.id,
            approver.id,
            now,
            Box::new(|_| panic!("not audited")),
        )
        .await
        .expect("second");
    assert!(matches!(again, ConfirmOutcome::NotFound));
    assert!(store
        .list_live_approval_confirmations(ws.id, now)
        .await
        .expect("live")
        .is_empty());

    // A link for a gate that has since resolved is refused as resolved, and
    // the gate keeps its first decision.
    let gate2 = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: None,
            requested_by: requester.id,
            prompt: "later?".into(),
            schema: None,
            risk: ApprovalRisk::Medium,
        })
        .await
        .expect("gate2");
    let nonce2 = Uuid::now_v7();
    let mut link2 = new(nonce2, "hash-3");
    link2.gate_id = gate2.id;
    store
        .issue_approval_confirmation(&link2, audit("issued", ws.id))
        .await
        .expect("issue2");
    store
        .resolve_approval_gate(gate2.id, approver.id, ApprovalGateState::Declined, None)
        .await
        .expect("decline");
    let outcome = store
        .confirm_approval_gate(
            "hash-3",
            ws.id,
            approver.id,
            now,
            Box::new(|_| panic!("not audited")),
        )
        .await
        .expect("confirm resolved");
    assert!(matches!(outcome, ConfirmOutcome::GateResolved));
    assert_eq!(
        store
            .get_approval_gate(gate2.id)
            .await
            .expect("get")
            .unwrap()
            .state,
        ApprovalGateState::Declined
    );

    // Another member, and another workspace, cannot spend a live link.
    let gate3 = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: None,
            requested_by: requester.id,
            prompt: "third?".into(),
            schema: None,
            risk: ApprovalRisk::Low,
        })
        .await
        .expect("gate3");
    let mut link3 = new(Uuid::now_v7(), "hash-4");
    link3.gate_id = gate3.id;
    store
        .issue_approval_confirmation(&link3, audit("issued", ws.id))
        .await
        .expect("issue3");
    let other = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "other".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("other");
    let outcome = store
        .confirm_approval_gate(
            "hash-4",
            ws.id,
            other.id,
            now,
            Box::new(|_| panic!("not audited")),
        )
        .await
        .expect("other member");
    assert!(matches!(outcome, ConfirmOutcome::NotFound));
    let other_ws = store
        .create_workspace(NewWorkspace {
            name: "elsewhere".into(),
        })
        .await
        .expect("other ws");
    let outcome = store
        .confirm_approval_gate(
            "hash-4",
            other_ws.id,
            approver.id,
            now,
            Box::new(|_| panic!("not audited")),
        )
        .await
        .expect("other workspace");
    assert!(matches!(outcome, ConfirmOutcome::NotFound));
    assert!(
        !store
            .get_approval_gate(gate3.id)
            .await
            .expect("get")
            .unwrap()
            .state
            .is_resolved(),
        "probing spent the link"
    );
}

#[tokio::test]
async fn a_confirmation_is_issued_once_spent_once_and_refused_when_expired_or_foreign_on_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn a_confirmation_is_issued_once_spent_once_and_refused_when_expired_or_foreign_on_postgres()
{
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
