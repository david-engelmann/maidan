//! A gate answered through `approval_decide` is announced as
//! `approval_resolved` on the gate's own workspace stream, once, and nowhere
//! else. `request_approval` on a thread the caller cannot read is not found
//! and writes nothing.

use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{
    capability::{MESSAGE_POST, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_mcp::{error::McpError, server::McpServer};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::{json, Value};

fn body(value: &Value) -> Value {
    serde_json::from_str(value["content"][0]["text"].as_str().unwrap()).unwrap()
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str, kind: MemberKind) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind,
        })
        .await
        .unwrap()
        .id
}

async fn thread(store: &dyn Store, ws: WorkspaceId, name: &str, private: bool) -> Thread {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: name.into(),
            topic: None,
            private,
        })
        .await
        .unwrap();
    store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some(name.into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
}

/// The next event on `stream`, or `None` when nothing arrives in a short
/// window.
async fn next_event<S>(stream: &mut S) -> Option<Event>
where
    S: futures::Stream<Item = BusItem> + Unpin,
{
    loop {
        match tokio::time::timeout(Duration::from_millis(300), stream.next()).await {
            Ok(Some(BusItem::Event(envelope))) => return Some(envelope.event),
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => return None,
        }
    }
}

#[tokio::test]
async fn approval_decide_announces_approval_resolved_to_the_gates_workspace_only() {
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
    let home = store
        .create_workspace(NewWorkspace {
            name: "home".into(),
        })
        .await
        .unwrap()
        .id;
    let away = store
        .create_workspace(NewWorkspace {
            name: "away".into(),
        })
        .await
        .unwrap()
        .id;
    let agent = member(store.as_ref(), home, "agent", MemberKind::Agent).await;
    let person = member(store.as_ref(), home, "person", MemberKind::Human).await;
    let stranger = member(store.as_ref(), away, "stranger", MemberKind::Human).await;
    let open = thread(store.as_ref(), home, "work", false).await;
    let private = thread(store.as_ref(), home, "board", true).await;

    let bus = Arc::new(InMemoryBus::new());
    let mut home_events = bus.subscribe(EventFilter::workspace(home)).await.unwrap();
    let mut away_events = bus.subscribe(EventFilter::workspace(away)).await.unwrap();
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(bus);
    let caps = vec![
        WORKSPACE_READ.to_string(),
        WORKSPACE_WRITE.to_string(),
        MESSAGE_POST.to_string(),
    ];
    let as_agent = AuthContext::from_session(agent, home, caps.clone());
    let as_person = AuthContext::from_session(person, home, caps.clone());
    let as_stranger = AuthContext::from_session(stranger, away, caps);

    // A private channel the agent is not in: not found, nothing written.
    let refused = server
        .call_tool(
            &as_agent,
            "request_approval",
            &json!({ "prompt": "ship the deck?", "thread_id": private.id.0 }),
        )
        .await;
    assert!(
        matches!(refused, Err(McpError::NotFound)),
        "a thread the caller cannot read must read as not found, got {refused:?}"
    );
    assert!(store
        .list_pending_approval_gates(home, None, 50)
        .await
        .unwrap()
        .is_empty());
    assert!(next_event(&mut home_events).await.is_none());

    let opened = body(
        &server
            .call_tool(
                &as_agent,
                "request_approval",
                &json!({ "prompt": "merge the fix?", "thread_id": open.id.0 }),
            )
            .await
            .unwrap(),
    );
    let gate_id = opened["gate_id"].clone();
    assert!(matches!(
        next_event(&mut home_events).await,
        Some(Event::ApprovalRequested { requested_by, .. }) if requested_by == agent
    ));

    // Another workspace's member cannot answer it, and nothing is announced.
    let crossed = server
        .call_tool(
            &as_stranger,
            "approval_decide",
            &json!({ "gate_id": gate_id, "decision": "decline" }),
        )
        .await;
    assert!(crossed.is_err(), "another workspace answered the gate");
    assert!(next_event(&mut home_events).await.is_none());

    let decided = body(
        &server
            .call_tool(
                &as_person,
                "approval_decide",
                &json!({ "gate_id": gate_id, "decision": "decline", "note": "not today" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(decided["status"], "declined");
    match next_event(&mut home_events).await {
        Some(Event::ApprovalResolved {
            workspace_id,
            channel_id,
            thread_id,
            gate_id: resolved,
            state,
            requested_by,
            resolved_by,
            ..
        }) => {
            assert_eq!(workspace_id, home);
            assert_eq!(channel_id, Some(open.channel_id));
            assert_eq!(thread_id, Some(open.id));
            assert_eq!(json!(resolved.0), gate_id);
            assert_eq!(state, ApprovalGateState::Declined);
            assert_eq!(requested_by, agent);
            assert_eq!(resolved_by, person);
        }
        other => panic!("expected approval_resolved, got {other:?}"),
    }

    // A second answer is already resolved and announces nothing.
    let again = body(
        &server
            .call_tool(
                &as_person,
                "approval_decide",
                &json!({ "gate_id": gate_id, "decision": "decline" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(again["status"], "already_resolved");
    assert!(next_event(&mut home_events).await.is_none());
    assert!(
        next_event(&mut away_events).await.is_none(),
        "another workspace heard this workspace's gate"
    );
}
