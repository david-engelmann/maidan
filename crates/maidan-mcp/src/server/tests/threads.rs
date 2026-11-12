//! Thread coordination: owner and steer, waits, declared status, threading,
//! budgets, spawning, snapshots, schedules and recipes.

use super::*;

#[tokio::test]
async fn owner_and_steer_tools_set_get_and_clear() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_bus::InMemoryBus;

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
        .create_workspace(NewWorkspace { name: "ow".into() })
        .await
        .unwrap();
    let mk_member = |handle: String, store: Arc<dyn Store>| async move {
        store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle,
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap()
    };
    let caller = mk_member("caller".into(), store.clone()).await;
    let owner = mk_member("owner".into(), store.clone()).await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
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

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    // A real session member so steered_by (a NOT-NULL FK) resolves.
    let auth = AuthContext::from_session(
        caller.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Set the owner.
    let owned = unwrap_content(
        server
            .call_tool(
                &auth,
                "set_thread_owner",
                &json!({ "thread_id": thread.id.0, "owner_id": owner.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(owned["owner_id"], json!(owner.id.0));

    // Clear it (omit owner_id).
    let cleared = unwrap_content(
        server
            .call_tool(
                &auth,
                "set_thread_owner",
                &json!({ "thread_id": thread.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert!(cleared["owner_id"].is_null());

    // No steer yet.
    assert!(unwrap_content(
        server
            .call_tool(
                &auth,
                "get_thread_steer",
                &json!({ "thread_id": thread.id.0 })
            )
            .await
            .unwrap()
    )
    .is_null());

    // Set + read the steer.
    server
        .call_tool(
            &auth,
            "set_thread_steer",
            &json!({ "thread_id": thread.id.0, "steer": "focus on the failing test" }),
        )
        .await
        .unwrap();
    let got = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_thread_steer",
                &json!({ "thread_id": thread.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(got["steer"], json!("focus on the failing test"));
    assert_eq!(got["steered_by"], json!(caller.id.0));

    // Rename: the new title round-trips; a blank title is rejected.
    let renamed = unwrap_content(
        server
            .call_tool(
                &auth,
                "rename_thread",
                &json!({ "thread_id": thread.id.0, "title": "renamed via mcp" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(renamed["title"], json!("renamed via mcp"));
    assert!(server
        .call_tool(
            &auth,
            "rename_thread",
            &json!({ "thread_id": thread.id.0, "title": "   " }),
        )
        .await
        .is_err());
}

/// The wait tools — set/get/cancel on the MCP path.
#[tokio::test]
async fn wait_tools_set_get_cancel() {
    use chrono::{Duration, Utc};
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
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
        .create_workspace(NewWorkspace { name: "wt".into() })
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
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    // Real member: set_wait persists auth.member_id (created_by FK).
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let body = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let until = (Utc::now() + Duration::hours(1)).to_rfc3339();

    // Get before set → null.
    assert!(body(
        server
            .call_tool(&auth, "get_wait", &json!({ "thread_id": thread.id.0 }))
            .await
            .unwrap()
    )
    .is_null());

    // Set a park wait.
    let set = body(
        server
            .call_tool(
                &auth,
                "set_wait",
                &json!({ "thread_id": thread.id.0, "wait_until": until, "on_timeout": "park", "reason": "blocked" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(set["on_timeout"], "park");
    assert_eq!(set["reason"], "blocked");

    // Get returns it.
    let got = body(
        server
            .call_tool(&auth, "get_wait", &json!({ "thread_id": thread.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(got["thread_id"], json!(thread.id.0));

    // Cancel → true, then false.
    assert_eq!(
        body(
            server
                .call_tool(&auth, "cancel_wait", &json!({ "thread_id": thread.id.0 }))
                .await
                .unwrap()
        )["cancelled"],
        json!(true)
    );
    assert_eq!(
        body(
            server
                .call_tool(&auth, "cancel_wait", &json!({ "thread_id": thread.id.0 }))
                .await
                .unwrap()
        )["cancelled"],
        json!(false)
    );
}

#[tokio::test]
async fn status_tools_declare_read_back_and_log_status_declared() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_types::{DeclaredStatus, Event};

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
        .create_workspace(NewWorkspace { name: "st".into() })
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
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parser".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    // Status is declared by the claim holder or the thread owner: make the
    // agent the owner so its declaration is accepted, mirroring the REST
    // e2e setup.
    store
        .set_thread_owner(thread.id, Some(agent.id))
        .await
        .unwrap();
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let body = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let declared = body(
        server
            .call_tool(
                &auth,
                "declare_status",
                &json!({ "thread_id": thread.id.0, "status": "needs_review", "note": "Ready for a look." }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(declared["status"], "needs_review");
    assert_eq!(declared["declared_by"], agent.id.0.to_string());
    let read = body(
        server
            .call_tool(
                &auth,
                "get_thread_status",
                &json!({ "thread_id": thread.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(read["status"], "needs_review");
    assert_eq!(read["note"], "Ready for a look.");

    let logged: Vec<DeclaredStatus> = store
        .list_events_after(ws.id, 0, 500)
        .await
        .unwrap()
        .iter()
        .filter_map(|e| match e.opened_event().ok()? {
            Event::StatusDeclared {
                thread_id, status, ..
            } if thread_id == thread.id => Some(status),
            _ => None,
        })
        .collect();
    assert_eq!(logged, vec![DeclaredStatus::NeedsReview]);

    assert!(
        server
            .call_tool(
                &auth,
                "declare_status",
                &json!({ "thread_id": thread.id.0, "status": "stalled", "note": "Stuck." }),
            )
            .await
            .is_err(),
        "stalled is system-computed and cannot be declared"
    );
}

/// A human DM reply posted over MCP clears the declaration on the DM's
/// backing thread, mirroring the REST DM post path.
#[tokio::test]
async fn mcp_human_dm_reply_clears_the_declared_status() {
    use maidan_auth::capability::{MESSAGE_POST, THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_types::MemberKind;

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
            name: "mcp-dm-clear".into(),
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
    let human = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "human".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let dm = store
        .open_dm_conversation(ws.id, agent.id, human.id)
        .await
        .unwrap();
    // Status is declared by the claim holder or the thread owner: make
    // the agent the owner so its declaration is accepted.
    store
        .set_thread_owner(dm.thread_id, Some(agent.id))
        .await
        .unwrap();
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let agent_auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let human_auth = AuthContext::from_session(
        human.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), MESSAGE_POST.to_string()],
    );
    let body = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // The agent declares.
    let declared = body(
        server
            .call_tool(
                &agent_auth,
                "declare_status",
                &json!({ "thread_id": dm.thread_id.0, "status": "working", "note": "Drafting the reply." }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(declared["status"], "working");

    // The human replies in the DM over MCP.
    server
        .call_tool(
            &human_auth,
            "post_dm_message",
            &json!({ "dm_conversation_id": dm.id.0, "body": "Got it, thanks." }),
        )
        .await
        .unwrap();

    // The declaration is gone.
    let read = body(
        server
            .call_tool(
                &agent_auth,
                "get_thread_status",
                &json!({ "thread_id": dm.thread_id.0 }),
            )
            .await
            .unwrap(),
    );
    assert!(
        read.is_null(),
        "a human DM reply over MCP should clear the declared status, got {read}"
    );
}

/// The threading MCP parity tools — collapsed children, recent activity,
/// and self leaf mute/unmute.
#[tokio::test]
async fn threading_tools_children_recent_and_mute() {
    use maidan_auth::capability::WORKSPACE_READ;
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
        .create_workspace(NewWorkspace { name: "th".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "m".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "threads".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let parent = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parent".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    // Gap so the child's post-bumped `updated_at` is strictly after the
    // parent's creation at SQLite's millisecond precision — otherwise a
    // same-millisecond tie makes the recent-activity order fall back to the
    // random-UUID tiebreak (flaky under compressed timing, e.g. llvm-cov).
    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    let child = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: Some(parent.id),
            title: Some("child".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .post_message(NewMessage {
            thread_id: child.id,
            author_id: member.id,
            body: "hi".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(member.id, ws.id, vec![WORKSPACE_READ.to_string()]);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Collapsed children: one child with a message count of 1.
    let children = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_child_threads",
                &json!({ "thread_id": parent.id.0 }),
            )
            .await
            .unwrap(),
    );
    let children = children.as_array().unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["message_count"], json!(1));
    assert_eq!(children[0]["thread"]["id"], json!(child.id.0));

    // Recent activity: both threads appear, child (posted-to) first.
    let recent = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_recently_active_threads",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap(),
    );
    let recent = recent.as_array().unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(
        recent[0]["id"],
        json!(child.id.0),
        "the posted-to thread bumps to the top"
    );

    // Leaf mute/unmute (self).
    let muted = unwrap_content(
        server
            .call_tool(&auth, "mute_thread", &json!({ "thread_id": child.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(muted["muted"], json!(true));
    assert!(store.is_thread_muted(member.id, child.id).await.unwrap());
    let unmuted = unwrap_content(
        server
            .call_tool(&auth, "unmute_thread", &json!({ "thread_id": child.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(unmuted["unmuted"], json!(true));
    let again = unwrap_content(
        server
            .call_tool(&auth, "unmute_thread", &json!({ "thread_id": child.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(again["unmuted"], json!(false), "second unmute is a no-op");
}

/// The budget MCP tools — set, report (stop on exceed), and read the DLQ.
#[tokio::test]
async fn budget_tools_set_report_and_dlq() {
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
        .create_workspace(NewWorkspace { name: "b".into() })
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
    let intruder = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "other-agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
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
    let claimed = store.assign_thread(thread.id, agent.id).await.unwrap();
    let claim_lease_id = claimed.claim_lease_id.expect("claim lease");

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec!["thread:transition".into(), "workspace:read".into()],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let set = unwrap_content(
        server
            .call_tool(
                &auth,
                "set_thread_budget",
                // A replace states every dimension; null is how you say "no
                // cap here".
                &json!({
                    "thread_id": thread.id.0,
                    "max_tokens": 100,
                    "max_usd_micros": null,
                    "max_turns": null,
                    "max_wall_secs": null
                }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(set["max_tokens"], json!(100));

    // Raising one dimension leaves the others alone. Through a replace this
    // would have cleared them, which is the defect.
    let raised = unwrap_content(
        server
            .call_tool(
                &auth,
                "update_thread_budget",
                &json!({ "thread_id": thread.id.0, "max_turns": 9 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(raised["max_turns"], json!(9));
    assert_eq!(raised["max_tokens"], json!(100), "the token cap survives");

    // A replace that omits a dimension is refused, not silently applied.
    assert!(
        server
            .call_tool(
                &auth,
                "set_thread_budget",
                &json!({ "thread_id": thread.id.0, "max_tokens": 1 }),
            )
            .await
            .is_err(),
        "a partial replace must be refused"
    );

    let usage = |tokens: i64| {
        json!({
            "thread_id": thread.id.0,
            "usage_report_id": uuid::Uuid::new_v4(),
            "claim_lease_id": claim_lease_id.0,
            "model": "test/model",
            "tokens": {
                "input": tokens,
                "output": 0,
                "cache_read": 0,
                "cache_write_5m": 0,
                "cache_write_1h": 0
            },
            "usd_micros": 0,
            "price_snapshot": {
                "input_usd_micros_per_million": 0,
                "output_usd_micros_per_million": 0,
                "cache_read_usd_micros_per_million": 0,
                "cache_write_5m_usd_micros_per_million": 0,
                "cache_write_1h_usd_micros_per_million": 0
            },
            "turns": 1
        })
    };
    let first = usage(50);
    let wrong_auth = AuthContext::from_session(
        intruder.id,
        ws.id,
        vec!["thread:transition".into(), "workspace:read".into()],
    );
    assert!(server
        .call_tool(&wrong_auth, "report_usage", &usage(1))
        .await
        .is_err());
    let under = unwrap_content(
        server
            .call_tool(&auth, "report_usage", &first)
            .await
            .unwrap(),
    );
    assert_eq!(under["stopped"], json!(false));
    assert_eq!(under["stamp"]["payer"], json!(ws.id.0));
    assert_eq!(under["stamp"]["reporter"], json!(agent.id.0));

    let retried = unwrap_content(
        server
            .call_tool(&auth, "report_usage", &first)
            .await
            .unwrap(),
    );
    assert_eq!(retried, under, "exact retry returns the first outcome");

    let over = unwrap_content(
        server
            .call_tool(&auth, "report_usage", &usage(60))
            .await
            .unwrap(),
    );
    assert_eq!(over["stopped"], json!(true));
    assert_eq!(over["reason"], json!("tokens"));
    assert_eq!(
        store.get_thread(thread.id).await.unwrap().assignee_id,
        None,
        "claim released on stop"
    );

    let dlq = unwrap_content(
        server
            .call_tool(&auth, "list_dlq", &json!({ "channel_id": channel.id.0 }))
            .await
            .unwrap(),
    );
    let dlq = dlq.as_array().unwrap();
    assert_eq!(dlq.len(), 1);
    assert_eq!(dlq[0]["reason"], json!("tokens"));
    assert_eq!(dlq[0]["member_id"], json!(agent.id.0));

    let got = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_thread_budget",
                &json!({ "thread_id": thread.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(got["used_tokens"], json!(110));
}

#[tokio::test]
async fn task_schedule_tools_create_and_list() {
    use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};

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
        .create_workspace(NewWorkspace { name: "sch".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "q".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    // A real member session so `created_by` satisfies its FK.
    let auth = AuthContext::from_session(
        member.id,
        ws.id,
        vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
    );
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let created = content(
        server
            .call_tool(
                &auth,
                "create_task_schedule",
                &json!({ "channel_id": channel.id.0, "title": "nightly", "interval_secs": 3600 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(created["title"], json!("nightly"));
    assert_eq!(created["interval_secs"], json!(3600));
    assert_eq!(created["active"], json!(true));

    let list = content(
        server
            .call_tool(&auth, "list_task_schedules", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], created["id"]);
}

#[tokio::test]
async fn recipe_tools_create_list_and_instantiate() {
    use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};

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
        .create_workspace(NewWorkspace { name: "rec".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "q".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    // A real member session so `created_by` satisfies its FK.
    let auth = AuthContext::from_session(
        member.id,
        ws.id,
        vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
    );
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let created = content(
        server
            .call_tool(
                &auth,
                "create_recipe",
                &json!({
                    "channel_id": channel.id.0,
                    "name": "ship",
                    "spec": {
                        "params": [{ "name": "repo", "required": true }],
                        "children": [
                            { "key": "build", "title": "build it", "required_skills": ["rust"] },
                            { "key": "review", "title": "review it", "depends_on": ["build"] }
                        ]
                    }
                }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(created["name"], json!("ship"));
    let recipe_id = created["id"].as_str().unwrap();

    let list = content(
        server
            .call_tool(&auth, "list_recipes", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);

    let run = content(
        server
            .call_tool(
                &auth,
                "instantiate_recipe",
                &json!({ "recipe_id": recipe_id, "params": { "repo": "x/y" } }),
            )
            .await
            .unwrap(),
    );
    // The run's root thread carries the two children.
    let root = ThreadId(uuid::Uuid::parse_str(run["root_thread_id"].as_str().unwrap()).unwrap());
    assert_eq!(store.child_thread_summaries(root).await.unwrap().len(), 2);

    // A missing required param is rejected.
    assert!(server
        .call_tool(
            &auth,
            "instantiate_recipe",
            &json!({ "recipe_id": recipe_id, "params": {} }),
        )
        .await
        .is_err());
}

#[tokio::test]
async fn spawn_budget_tools_set_get_and_gate_the_spawn() {
    use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};
    use maidan_store::StoreError;
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
        .create_workspace(NewWorkspace { name: "sb".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
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
    let parent = store
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
    let auth = AuthContext::from_session(
        member.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), WORKSPACE_WRITE.to_string()],
    );
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let child = || {
        let store = store.clone();
        let (channel_id, parent_thread_id) = (channel.id, parent.id);
        async move {
            store
                .create_thread(NewThread {
                    channel_id,
                    parent_thread_id: Some(parent_thread_id),
                    title: None,
                    description: None,
                    created_by: None,
                })
                .await
        }
    };

    // Unset = unlimited on every axis.
    let before = content(
        server
            .call_tool(&auth, "get_spawn_budget", &json!({}))
            .await
            .unwrap(),
    );
    assert!(before["max_children"].is_null());

    // Set the caller's own workspace budget; the read-back matches.
    let set = content(
        server
            .call_tool(
                &auth,
                "set_spawn_budget",
                &json!({ "max_children": 1, "max_depth": 3, "max_tools": 16 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(set["max_children"], 1);
    assert_eq!(set["max_depth"], 3);
    assert_eq!(set["max_tools"], 16);
    let read_back = content(
        server
            .call_tool(&auth, "get_spawn_budget", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(read_back, set, "get must read back what set stored");

    // The configured cap is the one the gate enforces.
    child()
        .await
        .expect("the first child is within max_children");
    let denied = child().await;
    assert!(
        matches!(&denied, Err(StoreError::SpawnRejected(d)) if d.axis == SpawnAxis::Children),
        "a 2nd child past max_children=1 must be refused, got {denied:?}"
    );

    // No arguments = a full replace with no caps → the budget is cleared and
    // spawning re-opens.
    server
        .call_tool(&auth, "set_spawn_budget", &json!({}))
        .await
        .unwrap();
    assert!(store.get_spawn_budget(ws.id).await.unwrap().is_none());
    child().await.expect("cleared budget re-opens spawning");

    // A negative axis is invalid params.
    assert!(server
        .call_tool(&auth, "set_spawn_budget", &json!({ "max_tools": -1 }))
        .await
        .is_err());
}

#[tokio::test]
async fn seed_from_message_tool_spawns_a_linked_child() {
    use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};

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
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "g".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("root".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let source = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "tangent".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    // A real member session so the quote's author FK is satisfied.
    let auth = AuthContext::from_session(
        member.id,
        ws.id,
        vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
    );
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let child = content(
        server
            .call_tool(
                &auth,
                "seed_from_message",
                &json!({ "message_id": source.id.0, "title": "branch", "inclusion": "quote" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(child["title"], json!("branch"));
    let child_id: uuid::Uuid = serde_json::from_value(child["id"].clone()).unwrap();

    // The seeded_from edge points from the child thread at the source message.
    let refs = store
        .list_references_to(maidan_types::RefSide::Message, source.id.0)
        .await
        .unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].relation, maidan_types::RelationKind::SeededFrom);
    assert_eq!(refs[0].src_id, child_id);

    // The quote inclusion posted a first message quoting the source.
    let msgs = store
        .list_messages(maidan_types::ThreadId(child_id), 10)
        .await
        .unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].body, "> tangent");
}

#[tokio::test]
async fn snapshot_thread_context_tool_freezes_and_refs() {
    use maidan_auth::capability::{ARTIFACT_UPLOAD, WORKSPACE_READ};

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
        .create_workspace(NewWorkspace { name: "cs".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "g".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "freeze me".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(
        member.id,
        ws.id,
        vec![ARTIFACT_UPLOAD.to_string(), WORKSPACE_READ.to_string()],
    );
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let artifact = content(
        server
            .call_tool(
                &auth,
                "snapshot_thread_context",
                &json!({ "thread_id": thread.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(artifact["kind"], json!("context_snapshot"));
    let sha = artifact["sha256"].as_str().unwrap().to_string();
    assert!(artifact["size_bytes"].as_i64().unwrap() > 0);

    // The access ref is recorded for the caller's workspace.
    assert!(store.artifact_ref_exists(ws.id, &sha).await.unwrap());

    // An identical snapshot dedups to the same sha.
    let again = content(
        server
            .call_tool(
                &auth,
                "snapshot_thread_context",
                &json!({ "thread_id": thread.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(again["sha256"].as_str().unwrap(), sha);
}
