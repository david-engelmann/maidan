//! Durable human-approval gates: create a pending gate, list it while
//! outstanding, resolve it to accept/decline/cancel, and prove the resolve is a
//! compare-and-set on `pending` (a double-answer is a no-op). Both backends. No
//! routes/tool yet — the zero-blast-radius foundation.

use chrono::Timelike;
use maidan_store::{prelude::*, run_sqlite_migrations, PendingGateQuery};
use maidan_types::{
    ApprovalGate, ApprovalGateState, MemberKind, NewApprovalGate, NewChannel, NewMember, NewThread,
    NewWorkspace,
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
    SqliteStore::for_tests(pool)
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
            description: None,
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
    let (gate, stored) = store
        .create_approval_gate_with_event(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: requester.id,
            prompt: "Deploy to prod?".into(),
            schema: Some(schema.clone()),
        })
        .await
        .expect("create");
    assert_eq!(stored.kind, maidan_types::EventKind::ApprovalRequested);
    assert_eq!(stored.workspace_id, Some(ws.id));
    assert_eq!(stored.channel_id, Some(channel.id));
    assert_eq!(stored.thread_id, Some(thread.id));
    let event: maidan_types::Event = serde_json::from_value(stored.payload).expect("event payload");
    assert!(matches!(
        event,
        maidan_types::Event::ApprovalRequested {
            gate_id,
            requested_by,
            ..
        } if gate_id == gate.id && requested_by == requester.id
    ));
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

/// `ListTasks` pages pending gates with the store's keyset: newest first,
/// ties by id descending, at millisecond precision, filtered by thread and
/// age, with resolved gates gone.
async fn run_paging_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "gate-pages".into(),
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
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "pages".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let mut threads = Vec::new();
    for _ in 0..2 {
        threads.push(
            store
                .create_thread(NewThread {
                    channel_id: channel.id,
                    parent_thread_id: None,
                    title: None,
                    description: None,
                })
                .await
                .expect("thread")
                .id,
        );
    }
    // Seven gates: four on the first thread, two on the second, one on none.
    // Created back to back, several share a millisecond.
    let mut gates: Vec<ApprovalGate> = Vec::new();
    for thread_id in [
        Some(threads[0]),
        Some(threads[1]),
        None,
        Some(threads[0]),
        Some(threads[0]),
        Some(threads[1]),
        Some(threads[0]),
    ] {
        gates.push(
            store
                .create_approval_gate(&NewApprovalGate {
                    workspace_id: ws.id,
                    thread_id,
                    requested_by: requester.id,
                    prompt: "ok?".into(),
                    schema: None,
                })
                .await
                .expect("gate"),
        );
    }
    let resolved = gates.remove(3);
    store
        .resolve_approval_gate(resolved.id, requester.id, ApprovalGateState::Accepted, None)
        .await
        .expect("resolve")
        .expect("was pending");
    for gate in &gates {
        assert_eq!(gate.created_at.nanosecond() % 1_000_000, 0, "whole ms");
    }
    let mut newest_first = gates.clone();
    newest_first.sort_by(|a, b| (b.created_at, b.id.0).cmp(&(a.created_at, a.id.0)));
    let ids = |gates: &[ApprovalGate]| gates.iter().map(|g| g.id).collect::<Vec<_>>();

    let page = |query: PendingGateQuery| store.page_pending_approval_gates(ws.id, query);
    let mut walked = Vec::new();
    let mut before = None;
    loop {
        let batch = page(PendingGateQuery {
            before,
            limit: 2,
            ..Default::default()
        })
        .await
        .expect("page");
        let Some(last) = batch.last() else { break };
        before = Some((last.created_at, Some(last.id)));
        walked.extend(ids(&batch));
    }
    assert_eq!(
        walked,
        ids(&newest_first),
        "every pending gate once, in order"
    );

    // `(at, None)` keeps only the gates opened before `at`.
    let pivot = &newest_first[2];
    let older = page(PendingGateQuery {
        before: Some((pivot.created_at, None)),
        limit: 50,
        ..Default::default()
    })
    .await
    .expect("before");
    let expected: Vec<_> = newest_first
        .iter()
        .filter(|g| g.created_at < pivot.created_at)
        .cloned()
        .collect();
    assert_eq!(ids(&older), ids(&expected));

    let first_thread = page(PendingGateQuery {
        thread_id: Some(threads[0]),
        limit: 50,
        ..Default::default()
    })
    .await
    .expect("thread");
    let expected: Vec<_> = newest_first
        .iter()
        .filter(|g| g.thread_id == Some(threads[0]))
        .cloned()
        .collect();
    assert_eq!(ids(&first_thread), ids(&expected));

    let since = pivot.created_at;
    let recent = page(PendingGateQuery {
        created_since: Some(since),
        limit: 50,
        ..Default::default()
    })
    .await
    .expect("since");
    let expected: Vec<_> = newest_first
        .iter()
        .filter(|g| g.created_at >= since)
        .cloned()
        .collect();
    assert_eq!(ids(&recent), ids(&expected));

    let counted = store
        .count_pending_approval_gates(ws.id, PendingGateQuery::default())
        .await
        .expect("count");
    assert_eq!(counted, 6);
    let counted = store
        .count_pending_approval_gates(
            ws.id,
            PendingGateQuery {
                thread_id: Some(threads[1]),
                created_since: Some(since),
                ..Default::default()
            },
        )
        .await
        .expect("count filtered");
    let expected = newest_first
        .iter()
        .filter(|g| g.thread_id == Some(threads[1]) && g.created_at >= since)
        .count();
    assert_eq!(counted, i64::try_from(expected).expect("small"));
}

#[tokio::test]
async fn approval_gate_create_list_resolve_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
    run_paging_suite(&store).await;
}

/// Migration 0116 rewrites every stored `created_at` to millisecond `...Z`
/// text, whichever form the row was written in, and leaves that form alone.
#[tokio::test]
async fn gate_timestamps_migrate_to_whole_milliseconds_sqlite() {
    let store = sqlite().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "legacy".into(),
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
    let written = [
        (
            "2026-09-28T20:13:05.123456789+00:00",
            "2026-09-28T20:13:05.123Z",
        ),
        (
            "2026-09-28T20:13:05.987654+00:00",
            "2026-09-28T20:13:05.987Z",
        ),
        ("2026-09-28T20:13:05.042+00:00", "2026-09-28T20:13:05.042Z"),
        ("2026-09-28T20:13:05+00:00", "2026-09-28T20:13:05.000Z"),
        ("2026-09-28 20:13:05", "2026-09-28T20:13:05.000Z"),
        ("2026-09-28T20:13:05.500Z", "2026-09-28T20:13:05.500Z"),
    ];
    let mut ids = Vec::new();
    for (created_at, _) in written {
        let gate = store
            .create_approval_gate(&NewApprovalGate {
                workspace_id: ws.id,
                thread_id: None,
                requested_by: requester.id,
                prompt: "ok?".into(),
                schema: None,
            })
            .await
            .expect("gate");
        sqlx::query("UPDATE maidan_approval_gates SET created_at = ? WHERE id = ?")
            .bind(created_at)
            .bind(gate.id.0)
            .execute(store.pool())
            .await
            .expect("backdate");
        ids.push(gate.id);
    }
    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/0116_approval_gate_millisecond_positions.sql"
    ))
    .execute(store.pool())
    .await
    .expect("migrate");
    for (id, (_, migrated)) in ids.into_iter().zip(written) {
        let stored: String =
            sqlx::query_scalar("SELECT created_at FROM maidan_approval_gates WHERE id = ?")
                .bind(id.0)
                .fetch_one(store.pool())
                .await
                .expect("read");
        assert_eq!(stored, migrated);
        let gate = store
            .get_approval_gate(id)
            .await
            .expect("get")
            .expect("gate");
        assert_eq!(
            gate.created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            migrated
        );
    }
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
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
    run_paging_suite(&store).await;
}
