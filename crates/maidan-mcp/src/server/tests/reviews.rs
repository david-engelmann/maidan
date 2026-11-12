//! Reviews and gates: approval gates, review requirements, FSM transitions
//! and land gates.

use super::*;

#[tokio::test]
async fn approval_gate_tools_request_and_poll() {
    use futures::StreamExt;
    use maidan_bus::{BusItem, EventBus, InMemoryBus};

    let pool = maidan_search::sqlite_pool_options()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let ws = store
        .create_workspace(NewWorkspace {
            name: "gates".into(),
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let bus = Arc::new(InMemoryBus::new());
    let mut events = bus.subscribe(EventFilter::workspace(ws.id)).await.unwrap();
    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(bus);
    // `request_approval` persists `requested_by = auth.member_id` (a real-member
    // FK), so this runs with a from_session auth — the nil-member bypass would
    // FK-fail.
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![
            maidan_auth::capability::WORKSPACE_READ.to_string(),
            maidan_auth::capability::MESSAGE_POST.to_string(),
        ],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Opening a gate returns input_required immediately — no blocking.
    let opened = unwrap_content(
        server
            .call_tool(
                &auth,
                "request_approval",
                &json!({ "prompt": "Deploy v9 to prod?" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(opened["status"], json!("input_required"));
    let gate_id = opened["gate_id"].as_str().unwrap().to_string();
    let BusItem::Event(opened_event) = events.next().await.unwrap() else {
        panic!("expected approval event");
    };
    assert!(matches!(
        opened_event.event,
        Event::ApprovalRequested {
            channel_id: None,
            thread_id: None,
            requested_by,
            ..
        } if requested_by == agent.id
    ));

    // The gate is queryable and pending (silence is not consent).
    let polled = unwrap_content(
        server
            .call_tool(&auth, "get_approval_gate", &json!({ "gate_id": gate_id }))
            .await
            .unwrap(),
    );
    assert_eq!(polled["state"], json!("pending"));
    assert_eq!(polled["prompt"], json!("Deploy v9 to prod?"));
    assert_eq!(polled["requested_by"], json!(agent.id.0));

    // An unknown gate id reads as null — no cross-workspace existence oracle.
    let missing = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_approval_gate",
                &json!({ "gate_id": uuid::Uuid::new_v4().to_string() }),
            )
            .await
            .unwrap(),
    );
    assert!(missing.is_null());

    // N6: a thread-attached gate carries the thread_id (the claim-gate store
    // behaviour is proven in the `approval_gate_claim` integration test).
    let channel = server
        .store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = server
        .store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let gated = unwrap_content(
        server
            .call_tool(
                &auth,
                "request_approval",
                &json!({ "prompt": "gate this thread", "thread_id": thread.id.0 }),
            )
            .await
            .unwrap(),
    );
    let BusItem::Event(gated_event) = events.next().await.unwrap() else {
        panic!("expected thread approval event");
    };
    assert!(matches!(
        gated_event.event,
        Event::ApprovalRequested {
            channel_id: Some(event_channel_id),
            thread_id: Some(event_thread_id),
            ..
        } if event_channel_id == channel.id && event_thread_id == thread.id
    ));
    let gate_on_thread = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_approval_gate",
                &json!({ "gate_id": gated["gate_id"] }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(gate_on_thread["thread_id"], json!(thread.id.0));
}

#[tokio::test]
async fn review_tools_set_require_name_submit_and_status() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_types::NewThread;

    let pool = maidan_search::sqlite_pool_options()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let ws = store
        .create_workspace(NewWorkspace { name: "rv".into() })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let reviewer = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "reviewer".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
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
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let caps = vec![THREAD_TRANSITION.to_string(), WORKSPACE_READ.to_string()];
    let op = AuthContext::from_session(owner.id, ws.id, caps.clone());
    let rev = AuthContext::from_session(reviewer.id, ws.id, caps);
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let tid = json!(thread.id.0);

    // Require 1 approval; name the reviewer; status is 0 of 1.
    server
        .call_tool(
            &op,
            "set_review_requirement",
            &json!({ "thread_id": tid, "required_count": 1 }),
        )
        .await
        .unwrap();
    let added = content(
        server
            .call_tool(
                &op,
                "add_reviewer",
                &json!({ "thread_id": tid, "member_id": reviewer.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(added["added"], json!(true));
    let s0 = content(
        server
            .call_tool(&op, "get_review_status", &json!({ "thread_id": tid }))
            .await
            .unwrap(),
    );
    assert_eq!(s0["required_count"], 1);
    assert_eq!(s0["approvals_met"], json!(false));

    // Handed to review with a result, the reviewer approves what it was
    // shown (as themselves) → status flips to met.
    store
        .set_thread_result_claiming(thread.id, owner.id, &json!({ "status": "done" }))
        .await
        .unwrap();
    store
        .transition_thread(thread.id, owner.id, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();
    let root = handed_root(&store, thread.id).await;
    server
        .call_tool(
            &rev,
            "submit_review",
            &json!({ "thread_id": tid, "decision": "approve", "note": "lgtm", "evidence_root": root }),
        )
        .await
        .unwrap();
    let s1 = content(
        server
            .call_tool(&op, "get_review_status", &json!({ "thread_id": tid }))
            .await
            .unwrap(),
    );
    assert_eq!(s1["approvals"], 1);
    assert_eq!(s1["approvals_met"], json!(true));
    let reviews = content(
        server
            .call_tool(&op, "list_reviews", &json!({ "thread_id": tid }))
            .await
            .unwrap(),
    );
    assert_eq!(reviews.as_array().unwrap().len(), 1);

    // A changed verdict replaces the current review; the history keeps both.
    server
        .call_tool(
            &rev,
            "submit_review",
            &json!({ "thread_id": tid, "decision": "request_changes", "note": "cover the empty case" }),
        )
        .await
        .unwrap();
    let history = content(
        server
            .call_tool(&op, "list_review_history", &json!({ "thread_id": tid }))
            .await
            .unwrap(),
    );
    let history = history.as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["decision"], "approve");
    assert_eq!(history[0]["note"], "lgtm");
    assert_eq!(history[1]["decision"], "request_changes");

    // Land-gate verdicts keep their history too.
    store
        .add_member_skill(reviewer.id, maidan_types::LAND_GATE_SKILL)
        .await
        .unwrap();
    for args in [
        json!({ "thread_id": tid, "status": "fail" }),
        json!({ "thread_id": tid, "status": "pass", "artifact_sha": "cafe" }),
    ] {
        server
            .call_tool(&rev, "set_land_gate", &args)
            .await
            .unwrap();
    }
    let gate = content(
        server
            .call_tool(&op, "list_land_gate_history", &json!({ "thread_id": tid }))
            .await
            .unwrap(),
    );
    let gate = gate.as_array().unwrap();
    assert_eq!(gate.len(), 2);
    assert_eq!(gate[0]["status"], "fail");
    assert_eq!(gate[0]["land"], "red");
    assert_eq!(gate[1]["status"], "pass");
    assert_eq!(gate[1]["artifact_sha"], "cafe");
}

/// MCP `transition_thread` is the twin of REST `POST
/// /threads/:id`. Same store path → same SoD + close-gate; the transition
/// publishes `ThreadStateChanged` via `publish_stored`.
#[tokio::test]
async fn transition_thread_tool_advances_fsm_and_honors_gates() {
    use std::collections::HashSet;
    use std::time::Duration;

    use futures::StreamExt;
    use maidan_auth::capability::THREAD_TRANSITION;
    use maidan_bus::{BusItem, EventBus, InMemoryBus};
    use maidan_types::{Event, EventFilter, EventKind, NewThread, ReviewDecision};

    let pool = maidan_search::sqlite_pool_options()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let ws = store
        .create_workspace(NewWorkspace { name: "fsm".into() })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let claimer = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "claimer".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let reviewer = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "reviewer".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
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
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .unwrap();
    store.assign_thread(thread.id, claimer.id).await.unwrap();

    let bus = Arc::new(InMemoryBus::new());
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(bus.clone());
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let caps = vec![THREAD_TRANSITION.to_string()];
    let owner_auth = AuthContext::from_session(owner.id, ws.id, caps.clone());
    let claimer_auth = AuthContext::from_session(claimer.id, ws.id, caps);
    let tid = json!(thread.id.0);
    let args = |action: &str| {
        json!({
            "thread_id": tid,
            "action": action,
        })
    };

    let unknown = server
        .call_tool(&owner_auth, "transition_thread", &args("merge"))
        .await;
    assert!(
        matches!(unknown, Err(McpError::InvalidParams(ref m)) if m.contains("unknown action")),
        "unknown action must be InvalidParams, got {unknown:?}"
    );

    let filter = EventFilter {
        workspace_id: Some(ws.id),
        kinds: Some(HashSet::from([EventKind::ThreadStateChanged])),
        ..EventFilter::default()
    };
    let mut stream = bus.subscribe(filter).await.unwrap();

    let started = content(
        server
            .call_tool(&owner_auth, "transition_thread", &args("start_review"))
            .await
            .unwrap(),
    );
    assert_eq!(started["state"], "in_review");

    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("timeout waiting for ThreadStateChanged")
        .expect("subscriber ended without event");
    let BusItem::Event(envelope) = event else {
        panic!("expected event, got lag or end");
    };
    match envelope.event {
        Event::ThreadStateChanged {
            from_state,
            to_state,
            ..
        } => {
            assert_eq!(from_state, ThreadState::Open);
            assert_eq!(to_state, ThreadState::InReview);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let sod = server
        .call_tool(&claimer_auth, "transition_thread", &args("close"))
        .await;
    assert!(
        matches!(sod, Err(McpError::InvalidParams(ref m)) if m.contains("separation of duties")),
        "claimer landing its own owned thread must be SoD-denied, got {sod:?}"
    );

    store.set_review_requirement(thread.id, 1).await.unwrap();
    let gated = server
        .call_tool(&owner_auth, "transition_thread", &args("close"))
        .await;
    assert!(
        matches!(gated, Err(McpError::InvalidParams(ref m)) if m.contains("review requirement")),
        "close without the required approval must be refused, got {gated:?}"
    );
    let notes = store.list_messages(thread.id, 20).await.unwrap();
    let note = notes
        .iter()
        .find(|m| m.metadata.get("notice").and_then(|v| v.as_str()) == Some("transition_refused"));
    let note = note.expect("the refused close is recorded on the thread");
    assert!(
        note.body.contains("review requirement not met"),
        "{}",
        note.body
    );
    assert_eq!(note.author_id, owner_auth.member_id());

    let root = handed_root(&store, thread.id).await;
    store
        .submit_review(
            thread.id,
            reviewer.id,
            ReviewDecision::Approve,
            None,
            Some(&root),
        )
        .await
        .unwrap();
    let closed = content(
        server
            .call_tool(&owner_auth, "transition_thread", &args("close"))
            .await
            .unwrap(),
    );
    assert_eq!(closed["state"], "closed");
}

#[tokio::test]
async fn land_gate_tools_require_set_get_and_clear() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_types::{NewThread, LAND_GATE_SKILL};

    let pool = maidan_search::sqlite_pool_options()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let ws = store
        .create_workspace(NewWorkspace { name: "sc".into() })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let checker = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "land_gate".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    store
        .add_member_skill(checker.id, LAND_GATE_SKILL)
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
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .unwrap();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let caps = vec![THREAD_TRANSITION.to_string(), WORKSPACE_READ.to_string()];
    let op = AuthContext::from_session(owner.id, ws.id, caps.clone());
    let sc = AuthContext::from_session(checker.id, ws.id, caps);
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let tid = json!(thread.id.0);

    let pending = content(
        server
            .call_tool(&op, "require_land_gate", &json!({ "thread_id": tid }))
            .await
            .unwrap(),
    );
    assert_eq!(pending["required"], json!(true));
    assert_eq!(pending["landable"], json!(false));
    assert_eq!(pending["land"], "red");

    let amber = content(
        server
            .call_tool(
                &sc,
                "set_land_gate",
                &json!({
                    "thread_id": tid,
                    "status": "pass",
                    "land": "amber",
                    "artifact_sha": "abc"
                }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(amber["land"], "amber");
    assert_eq!(amber["landable"], json!(false));
    assert_eq!(amber["pointer"]["kind"], "land_gate");
    assert_eq!(amber["pointer"]["artifact_sha"], "abc");

    let green = content(
        server
            .call_tool(
                &sc,
                "set_land_gate",
                &json!({ "thread_id": tid, "status": "pass" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(green["land"], "green");
    assert_eq!(green["landable"], json!(true));

    let got = content(
        server
            .call_tool(&op, "get_land_gate", &json!({ "thread_id": tid }))
            .await
            .unwrap(),
    );
    assert_eq!(got["landable"], json!(true));

    // The gate ratchets. `op` holds `thread:transition` — the capability a
    // close needs — so it must not be able to delete the gate that
    // constrains the close.
    assert!(
        server
            .call_tool(&op, "clear_land_gate", &json!({ "thread_id": tid }))
            .await
            .is_err(),
        "a transitioner must not clear the gate constraining it"
    );
    let admin = AuthContext::from_session(
        owner.id,
        ws.id,
        vec![
            THREAD_TRANSITION.to_string(),
            WORKSPACE_READ.to_string(),
            maidan_auth::capability::CHANNEL_ADMIN.to_string(),
        ],
    );
    let cleared = content(
        server
            .call_tool(&admin, "clear_land_gate", &json!({ "thread_id": tid }))
            .await
            .unwrap(),
    );
    assert_eq!(cleared["cleared"], json!(true));
}

/// Every `submit_review` verdict publishes `ReviewSubmitted`, naming the
/// reviewer and the thread's last worker; a change request that sends the
/// work back publishes it before the reopen.
#[tokio::test]
async fn submit_review_tool_publishes_review_submitted() {
    use std::time::Duration;

    use futures::StreamExt;
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_bus::{BusItem, InMemoryBus};
    use maidan_fsm::ThreadAction;
    use maidan_types::ReviewDecision;

    let pool = maidan_search::sqlite_pool_options()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let ws = store
        .create_workspace(NewWorkspace { name: "rv".into() })
        .await
        .unwrap();
    let mut members = Vec::new();
    for handle in ["worker", "reviewer"] {
        members.push(
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
    let (worker, reviewer) = (members[0], members[1]);
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let claimed = store
        .claim_next_thread(channel.id, worker, Some(60), None)
        .await
        .unwrap()
        .unwrap();
    store
        .transition_thread(thread.id, worker, ThreadAction::StartReview)
        .await
        .unwrap();
    store
        .release_claim(thread.id, worker, claimed.claim_lease_id.unwrap())
        .await
        .unwrap();

    let bus = Arc::new(InMemoryBus::new());
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(bus.clone());
    let mut stream = bus
        .subscribe(
            EventFilter::thread(thread.id)
                .with_kinds([EventKind::ReviewSubmitted, EventKind::ThreadStateChanged]),
        )
        .await
        .unwrap();
    let rev = AuthContext::from_session(
        reviewer,
        ws.id,
        vec![THREAD_TRANSITION.to_string(), WORKSPACE_READ.to_string()],
    );
    async fn next(stream: &mut maidan_bus::EventStream) -> Event {
        match tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("timeout waiting for a review event")
            .expect("stream ended")
        {
            BusItem::Event(envelope) => envelope.event,
            BusItem::Lagged { skipped } => panic!("lagged by {skipped}"),
        }
    }

    // A change request that does not say what to change is refused.
    let refused = server
        .call_tool(
            &rev,
            "submit_review",
            &json!({ "thread_id": thread.id.0, "decision": "request_changes", "note": "  " }),
        )
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("needs a note"), "{refused}");

    for (decision, note, sent) in [
        ("request_changes", Some("handle the empty list"), true),
        ("approve", None, false),
    ] {
        let root = handed_root(&store, thread.id).await;
        server
            .call_tool(
                &rev,
                "submit_review",
                &json!({ "thread_id": thread.id.0, "decision": decision, "note": note, "evidence_root": root }),
            )
            .await
            .unwrap();
        match next(&mut stream).await {
            Event::ReviewSubmitted {
                thread_id,
                reviewer_id,
                decision: got,
                sent_back,
                worker_id,
                ..
            } => {
                assert_eq!(thread_id, thread.id);
                assert_eq!(reviewer_id, reviewer);
                assert_eq!(got.as_str(), decision);
                assert_eq!(sent_back, sent);
                assert_eq!(worker_id, Some(worker));
            }
            other => panic!("expected ReviewSubmitted, got {other:?}"),
        }
        if sent {
            assert!(
                matches!(next(&mut stream).await, Event::ThreadStateChanged { .. }),
                "the reopen follows the verdict"
            );
        }
    }
    assert_eq!(
        store.list_review_history(thread.id).await.unwrap()[1].decision,
        ReviewDecision::Approve
    );
}
