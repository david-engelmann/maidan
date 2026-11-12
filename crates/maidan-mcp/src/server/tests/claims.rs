//! Claiming work: assignment reads, `claim_next` and its lease, queue depth,
//! WIP limits, priorities, and parking a thread.

use super::*;

#[tokio::test]
async fn mcp_assignment_read_side_claims_lists_and_filters() {
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
        .create_workspace(NewWorkspace { name: "aq".into() })
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
    let public = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "open".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let secret = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "secret".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    let mk_thread = |channel_id, store: Arc<dyn Store>| async move {
        store
            .create_thread(NewThread {
                channel_id,
                parent_thread_id: None,
                title: Some("t".into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap()
    };
    let pub_thread = mk_thread(public.id, store.clone()).await;
    let secret_thread = mk_thread(secret.id, store.clone()).await;
    // The agent is assigned a thread in a private channel it is NOT a member
    // of (assignment is orthogonal to membership).
    store
        .assign_thread(secret_thread.id, agent.id)
        .await
        .unwrap();

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // claim_next takes the oldest unassigned thread in the public channel.
    let claimed = unwrap_content(
        server
            .call_tool(
                &auth,
                "claim_next_thread",
                &json!({ "channel_id": public.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(claimed["id"], json!(pub_thread.id.0));
    assert!(
        claimed["pin"]["uri"]
            .as_str()
            .unwrap()
            .starts_with("maidan:event/"),
        "claim pins the assignment event"
    );
    assert!(claimed["pin"]["content_hash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));

    // list_assigned shows the public-channel assignment, but the private one
    // is filtered out — the agent isn't a member of that channel.
    let queue = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_assigned_threads",
                &json!({ "member_id": agent.id.0 }),
            )
            .await
            .unwrap(),
    );
    let ids: Vec<Value> = queue
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].clone())
        .collect();
    assert!(
        ids.contains(&json!(pub_thread.id.0)),
        "public assignment visible"
    );
    assert!(
        !ids.contains(&json!(secret_thread.id.0)),
        "private-channel assignment filtered from a non-member caller"
    );

    // No unassigned work left in the public channel → null.
    let none = unwrap_content(
        server
            .call_tool(
                &auth,
                "claim_next_thread",
                &json!({ "channel_id": public.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert!(none.is_null());
}

/// A waiting workspace claim takes work filed while it waits, waking on
/// the thread's creation rather than polling; with nothing filed it
/// answers null at its deadline, and without `wait_ms` it does not wait.
#[tokio::test]
async fn a_waiting_claim_takes_work_filed_while_it_waits() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_bus::InMemoryBus;
    use std::time::{Duration, Instant};

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
            name: "wait".into(),
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "session".into(),
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
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let claim =
        |wait_ms: i64| json!({ "workspace_id": ws.id.0, "engine": "session", "wait_ms": wait_ms });

    // Nothing claimable and no wait: null at once.
    let started = Instant::now();
    let no_wait = json!({ "workspace_id": ws.id.0 });
    let none = server
        .call_tool(&auth, "claim_next_workspace_thread", &no_wait)
        .await
        .unwrap();
    assert!(unwrap_content(none).is_null());
    assert!(started.elapsed() < Duration::from_millis(500));

    // Unattended work filed during the wait does not end a session
    // claimant's wait; the session work filed after it is what the claim takes.
    let file = |title: &'static str, engine: TaskEngine| {
        let store = store.clone();
        let server = &server;
        async move {
            let (_, stored) = store
                .create_thread_with_footprint(
                    NewThread {
                        channel_id: channel.id,
                        parent_thread_id: None,
                        title: Some(title.into()),
                        description: None,
                        created_by: None,
                    },
                    None,
                    Some(TaskAttributes {
                        engine: Some(engine),
                        ..TaskAttributes::default()
                    }),
                    None,
                )
                .await
                .unwrap();
            server.publish_stored(&stored).await;
        }
    };
    let waiting = claim(5_000);
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "claim_next_workspace_thread", &waiting),
        async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            file("unattended work", TaskEngine::Unattended).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            file("session work", TaskEngine::Session).await;
        }
    );
    let got = unwrap_content(out.unwrap());
    assert_eq!(got["title"], json!("session work"), "{got}");
    assert_eq!(got["assignee_id"], json!(agent.id.0));

    // Waiting with nothing it may claim answers null at its deadline,
    // though the unattended work is still queued.
    let started = Instant::now();
    let short = claim(300);
    let out = server
        .call_tool(&auth, "claim_next_workspace_thread", &short)
        .await
        .unwrap();
    assert!(unwrap_content(out).is_null());
    assert!(started.elapsed() >= Duration::from_millis(250));
}

#[tokio::test]
async fn get_queue_depth_tool_reports_counts() {
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
        .create_workspace(NewWorkspace { name: "qd".into() })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: None,
    };
    // t1 has no deps → ready; t2 depends on t1 → blocked.
    let t1 = store.create_thread(mk("t1")).await.unwrap();
    let t2 = store.create_thread(mk("t2")).await.unwrap();
    store.add_thread_dependency(t2.id, t1.id).await.unwrap();

    let reader = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "reader".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = admin_auth(reader.id, ws.id);
    let out = server
        .call_tool(
            &auth,
            "get_queue_depth",
            &json!({ "channel_id": channel.id.0 }),
        )
        .await
        .unwrap();
    let depth: Value = serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(depth["open"], json!(2));
    assert_eq!(depth["ready"], json!(1), "t1 has no deps");
    assert_eq!(depth["blocked"], json!(1), "t2 waits on t1");
    assert_eq!(depth["assigned"], json!(0));
}

/// The priority tools — set/get on the MCP path.
#[tokio::test]
async fn priority_tools_set_and_get() {
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
        .create_workspace(NewWorkspace { name: "pr".into() })
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
    // Real member: set_priority persists auth.member_id (set_by FK).
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let body = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Get before set → null (= default 0).
    assert!(body(
        server
            .call_tool(&auth, "get_priority", &json!({ "thread_id": thread.id.0 }))
            .await
            .unwrap()
    )
    .is_null());

    // Set a priority.
    let set = body(
        server
            .call_tool(
                &auth,
                "set_priority",
                &json!({ "thread_id": thread.id.0, "priority": 8 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(set["priority"], json!(8));
    assert_eq!(set["thread_id"], json!(thread.id.0));

    // Get returns it.
    let got = body(
        server
            .call_tool(&auth, "get_priority", &json!({ "thread_id": thread.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(got["priority"], json!(8));
}

/// The unclaimable tools — park/un-park/list + the explicit claim refusal +
/// claim_next skip on the MCP path.
#[tokio::test]
async fn unclaimable_tools_park_a_thread_from_dispatch() {
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
        .create_workspace(NewWorkspace { name: "un".into() })
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
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: None,
    };
    let t1 = store.create_thread(mk("t1")).await.unwrap();
    let t2 = store.create_thread(mk("t2")).await.unwrap();

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    // Real member: mark_unclaimable persists auth.member_id (marked_by FK).
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let body = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Park t1.
    let marked = body(
        server
            .call_tool(
                &auth,
                "mark_unclaimable",
                &json!({ "thread_id": t1.id.0, "reason": "triage" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(marked["reason"], "triage");
    // The channel list shows it.
    let listed = body(
        server
            .call_tool(
                &auth,
                "list_unclaimable",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(listed.as_array().unwrap().len(), 1);

    // An explicit claim of the parked thread errors.
    assert!(server
        .call_tool(&auth, "claim_thread", &json!({ "thread_id": t1.id.0 }),)
        .await
        .is_err());
    // claim_next skips t1 and claims t2.
    let next = body(
        server
            .call_tool(
                &auth,
                "claim_next_thread",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(next["id"], json!(t2.id.0));

    // Un-park → cleared true, then false; now claimable.
    assert_eq!(
        body(
            server
                .call_tool(&auth, "mark_claimable", &json!({ "thread_id": t1.id.0 }))
                .await
                .unwrap()
        )["cleared"],
        json!(true)
    );
    let claimed = body(
        server
            .call_tool(&auth, "claim_thread", &json!({ "thread_id": t1.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(claimed["claimed"], json!(true));
}

/// The block tools — set/get/list + explicit-claim refusal + claim_next
/// skip + BlockedResolved on clear.
#[tokio::test]
async fn block_tools_park_a_thread_from_dispatch() {
    use std::time::Duration;

    use futures::StreamExt;
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
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
        .create_workspace(NewWorkspace { name: "blk".into() })
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
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: None,
    };
    let t1 = store.create_thread(mk("t1")).await.unwrap();
    let t2 = store.create_thread(mk("t2")).await.unwrap();

    let bus = Arc::new(InMemoryBus::new());
    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(bus.clone());
    let mut stream = bus
        .subscribe(EventFilter::all().with_kinds([EventKind::BlockedResolved]))
        .await
        .unwrap();
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let body = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let marked = body(
        server
            .call_tool(
                &auth,
                "set_thread_block",
                &json!({ "thread_id": t1.id.0, "reason": "human" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(marked["reason"], "human");
    assert_eq!(
        body(
            server
                .call_tool(&auth, "get_thread_block", &json!({ "thread_id": t1.id.0 }))
                .await
                .unwrap()
        )["reason"],
        "human"
    );
    let listed = body(
        server
            .call_tool(
                &auth,
                "list_blocked_threads",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(listed.as_array().unwrap().len(), 1);

    assert!(
        server
            .call_tool(
                &auth,
                "set_thread_block",
                &json!({ "thread_id": t2.id.0, "reason": "nope" }),
            )
            .await
            .is_err(),
        "closed enum rejects unknown reason"
    );

    assert!(server
        .call_tool(&auth, "claim_thread", &json!({ "thread_id": t1.id.0 }),)
        .await
        .is_err());
    let next = body(
        server
            .call_tool(
                &auth,
                "claim_next_thread",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(next["id"], json!(t2.id.0));

    assert_eq!(
        body(
            server
                .call_tool(
                    &auth,
                    "clear_thread_block",
                    &json!({ "thread_id": t1.id.0 })
                )
                .await
                .unwrap()
        )["cleared"],
        json!(true)
    );
    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("timeout waiting for BlockedResolved")
        .expect("subscriber ended");
    let BusItem::Event(envelope) = event else {
        panic!("expected event, got lag/end");
    };
    match envelope.event {
        Event::BlockedResolved {
            thread_id,
            reason,
            resolved_by,
            ..
        } => {
            assert_eq!(thread_id, t1.id);
            assert_eq!(reason, BlockedReason::Human);
            assert_eq!(resolved_by, agent.id);
        }
        other => panic!("unexpected event: {other:?}"),
    }
    let claimed = body(
        server
            .call_tool(&auth, "claim_thread", &json!({ "thread_id": t1.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(claimed["claimed"], json!(true));
}

/// The WIP admin/visibility tools + enforcement on the MCP claim path
/// (explicit claim errors at the cap; claim_next returns null).
#[tokio::test]
async fn wip_limit_tools_enforce_the_cap() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ, WORKSPACE_WRITE};
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
        .create_workspace(NewWorkspace { name: "wip".into() })
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
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: None,
    };
    let t1 = store.create_thread(mk("t1")).await.unwrap();
    let t2 = store.create_thread(mk("t2")).await.unwrap();

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![
            WORKSPACE_READ.to_string(),
            WORKSPACE_WRITE.to_string(),
            THREAD_TRANSITION.to_string(),
        ],
    );
    let body = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Cap the workspace at one live claim per member.
    let set = server
        .call_tool(&auth, "set_wip_limit", &json!({ "limit": 1 }))
        .await
        .unwrap();
    assert_eq!(body(set)["limit"], json!(1));
    assert_eq!(
        body(
            server
                .call_tool(&auth, "get_wip_limit", &json!({}))
                .await
                .unwrap()
        )["limit"],
        json!(1)
    );
    let wip0 = body(
        server
            .call_tool(&auth, "get_member_wip", &json!({ "member_id": agent.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(wip0["live_claims"], json!(0));
    assert_eq!(wip0["limit"], json!(1));

    // First claim OK; a second explicit claim errors at the cap.
    server
        .call_tool(&auth, "claim_thread", &json!({ "thread_id": t1.id.0 }))
        .await
        .unwrap();
    assert!(
        server
            .call_tool(&auth, "claim_thread", &json!({ "thread_id": t2.id.0 }),)
            .await
            .is_err(),
        "a second claim past the WIP cap errors"
    );
    // claim_next hands out nothing while at the cap.
    let next = body(
        server
            .call_tool(
                &auth,
                "claim_next_thread",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert!(next.is_null());

    // Clearing the cap lets the second claim through.
    server
        .call_tool(&auth, "set_wip_limit", &json!({ "limit": Value::Null }))
        .await
        .unwrap();
    let claimed = body(
        server
            .call_tool(&auth, "claim_thread", &json!({ "thread_id": t2.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(claimed["claimed"], json!(true));
}

#[tokio::test]
async fn mcp_claim_next_reclaim_emits_claim_expired_then_assignment() {
    // The MCP claim_next path goes through the
    // atomic `claim_next_thread_with_event` + `publish_stored`, so
    // reclaiming a dead holder's expired lease emits ClaimExpired (for the
    // holder) THEN ThreadAssignmentChanged — parity with REST. The old
    // non-event path dropped ClaimExpired on the agent surface entirely.
    use futures::StreamExt;
    use maidan_auth::capability::THREAD_TRANSITION;
    use maidan_bus::{BusItem, EventBus, InMemoryBus};
    use std::collections::HashSet;
    use std::time::Duration;

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
        .create_workspace(NewWorkspace { name: "rc".into() })
        .await
        .unwrap();
    let m1 = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "dead".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let m2 = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "reclaimer".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
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
    let text = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // m1 holds the thread on a lease that has already lapsed (a dead
    // agent). The tools refuse a lease under a second, so the store
    // writes it directly.
    let claimed1 = store
        .claim_next_thread(ch.id, m1.id, Some(-1), None)
        .await
        .unwrap();
    assert!(claimed1.is_some(), "m1 claimed the thread");

    // Subscribe BEFORE the reclaim so both events are captured.
    let filter = EventFilter {
        workspace_id: Some(ws.id),
        kinds: Some(HashSet::from([
            EventKind::ClaimExpired,
            EventKind::ThreadAssignmentChanged,
        ])),
        ..EventFilter::default()
    };
    let mut stream = bus.subscribe(filter).await.unwrap();

    // m2 reclaims the expired lease via the MCP tool.
    let auth2 = AuthContext::from_session(m2.id, ws.id, vec![THREAD_TRANSITION.to_string()]);
    let reclaimed = text(
        server
            .call_tool(
                &auth2,
                "claim_next_thread",
                &json!({ "channel_id": ch.id.0, "lease_secs": 3600 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        reclaimed["assignee_id"],
        m2.id.0.to_string(),
        "m2 reclaimed the thread"
    );

    // The reclaim emitted ClaimExpired (for the dead m1) + ThreadAssignmentChanged (m2).
    let mut events = vec![];
    for _ in 0..2 {
        match tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
            Ok(Some(BusItem::Event(env))) => events.push(env.event),
            _ => break,
        }
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::ClaimExpired { member_id, .. } if *member_id == m1.id)),
        "the MCP reclaim must emit ClaimExpired for the dead holder (the P1.1c fix); got {events:?}"
    );
    assert!(
        events.iter().any(
            |e| matches!(e, Event::ThreadAssignmentChanged { assignee_id, .. } if *assignee_id == Some(m2.id))
        ),
        "the MCP reclaim must emit ThreadAssignmentChanged for the reclaimer"
    );
}

#[tokio::test]
async fn claim_next_without_a_lease_gets_the_server_default_and_bounds_hold() {
    use maidan_auth::capability::THREAD_TRANSITION;

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
            name: "lease".into(),
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
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    for title in ["one", "two"] {
        store
            .create_thread(NewThread {
                channel_id: ch.id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap();
    }
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_claim_lease_policy(crate::claim_lease::ClaimLeasePolicy::new(900).unwrap());
    let auth = AuthContext::from_session(agent.id, ws.id, vec![THREAD_TRANSITION.to_string()]);
    let text = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // A lease out of bounds is refused before anything is claimed.
    for bad in [0, -1, crate::claim_lease::MAX_CLAIM_LEASE_SECS + 1] {
        let err = server
            .call_tool(
                &auth,
                "claim_next_thread",
                &json!({ "channel_id": ch.id.0, "lease_secs": bad }),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, McpError::InvalidParams(ref m) if m.contains("lease_secs")),
            "lease {bad}: {err:?}"
        );
    }
    assert!(
        store
            .list_threads(ch.id)
            .await
            .unwrap()
            .iter()
            .all(|t| t.assignee_id.is_none()),
        "a refused claim takes nothing"
    );

    // No lease named: the claim is leased for the server default.
    let before = chrono::Utc::now();
    let claimed = text(
        server
            .call_tool(
                &auth,
                "claim_next_thread",
                &json!({ "channel_id": ch.id.0 }),
            )
            .await
            .unwrap(),
    );
    let expires: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(claimed["assignment_expires_at"].clone()).unwrap();
    let lease = (expires - before).num_seconds();
    assert!((899..=901).contains(&lease), "default lease was {lease} s");
    assert!(
        claimed["claim_lease_id"].is_string(),
        "a leased claim is fenced"
    );

    // A named lease inside the bounds is kept.
    let before = chrono::Utc::now();
    let claimed = text(
        server
            .call_tool(
                &auth,
                "claim_next_thread",
                &json!({ "channel_id": ch.id.0, "lease_secs": 30 }),
            )
            .await
            .unwrap(),
    );
    let expires: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(claimed["assignment_expires_at"].clone()).unwrap();
    assert!((29..=31).contains(&(expires - before).num_seconds()));

    // Renewals are held to the same bounds.
    let err = server
        .call_tool(
            &auth,
            "renew_claim",
            &json!({
                "thread_id": claimed["id"],
                "claim_lease_id": claimed["claim_lease_id"],
                "lease_secs": 0,
            }),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::InvalidParams(_)), "{err:?}");
}
