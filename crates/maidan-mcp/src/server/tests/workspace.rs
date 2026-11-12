//! Workspace knowledge: projector links, memory blocks, the glossary and
//! skills.

use super::*;

#[tokio::test]
async fn projector_link_tools_slack_and_github() {
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
        .create_workspace(NewWorkspace { name: "p".into() })
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
    // A real workspace-scoped session: `list_*` reads `auth.workspace_id`, and
    // link resolves the thread's own workspace/channel (public → member has access).
    let auth = AuthContext::from_session(
        member.id,
        ws.id,
        vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Slack: link resolves ws/channel from the thread → list → unlink.
    let link = unwrap_content(
        server
            .call_tool(
                &auth,
                "link_slack_channel",
                &json!({ "thread_id": thread.id.0, "slack_channel_id": "C123" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(link["slack_channel_id"], json!("C123"));
    assert_eq!(link["channel_id"], json!(channel.id.0));
    let list = unwrap_content(
        server
            .call_tool(&auth, "list_slack_channel_links", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    let un = unwrap_content(
        server
            .call_tool(
                &auth,
                "unlink_slack_channel",
                &json!({ "slack_channel_id": "C123" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(un["unlinked"], json!(true));
    assert!(unwrap_content(
        server
            .call_tool(&auth, "list_slack_channel_links", &json!({}))
            .await
            .unwrap()
    )
    .as_array()
    .unwrap()
    .is_empty());

    // GitHub: same round-trip.
    let gh = unwrap_content(
        server
            .call_tool(
                &auth,
                "link_github_issue",
                &json!({ "thread_id": thread.id.0, "repo": "o/r", "issue_number": 7 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(gh["repo"], json!("o/r"));
    assert_eq!(gh["issue_number"], json!(7));
    let ghlist = unwrap_content(
        server
            .call_tool(&auth, "list_github_issue_links", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(ghlist.as_array().unwrap().len(), 1);
    let ghun = unwrap_content(
        server
            .call_tool(
                &auth,
                "unlink_github_issue",
                &json!({ "repo": "o/r", "issue_number": 7 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(ghun["unlinked"], json!(true));
}

#[tokio::test]
async fn memory_block_tools_create_read_rewrite_and_attach() {
    use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};
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
        .create_workspace(NewWorkspace { name: "mb".into() })
        .await
        .unwrap();
    // A real member: create_memory_block stamps owner_id (a NOT-NULL FK), so
    // the nil-member bypass would FK-fail — use a from_session auth.
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
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
    let auth = AuthContext::from_session(
        owner.id,
        ws.id,
        vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
    );
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Create + get by label.
    let created = content(
        server
            .call_tool(
                &auth,
                "create_memory_block",
                &json!({ "label": "shared", "char_limit": 10, "value": "hello" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(created["value"], "hello");
    let block_id = created["id"].as_str().unwrap().to_string();
    let got = content(
        server
            .call_tool(&auth, "get_memory_block", &json!({ "label": "shared" }))
            .await
            .unwrap(),
    );
    assert_eq!(got["value"], "hello");
    // list.
    let list = content(
        server
            .call_tool(&auth, "list_memory_blocks", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Full-rewrite by label.
    let updated = content(
        server
            .call_tool(
                &auth,
                "set_memory_block_value",
                &json!({ "label": "shared", "value": "world" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(updated["value"], "world");
    // Over the char limit → error.
    assert!(server
        .call_tool(
            &auth,
            "set_memory_block_value",
            &json!({ "label": "shared", "value": "this is way too long" }),
        )
        .await
        .is_err());

    // Attach by label → the thread lists it → detach by id.
    let attached = content(
        server
            .call_tool(
                &auth,
                "attach_memory_block",
                &json!({ "thread_id": thread.id.0, "label": "shared" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(attached["attached"], json!(true));
    let on_thread = content(
        server
            .call_tool(
                &auth,
                "list_thread_memory_blocks",
                &json!({ "thread_id": thread.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(on_thread.as_array().unwrap().len(), 1);
    let detached = content(
        server
            .call_tool(
                &auth,
                "detach_memory_block",
                &json!({ "thread_id": thread.id.0, "block_id": block_id }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(detached["detached"], json!(true));
}

#[tokio::test]
async fn memory_block_wait_wakes_on_update() {
    use maidan_auth::capability::{WORKSPACE_READ, WORKSPACE_WRITE};
    use maidan_bus::InMemoryBus;
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
        .create_workspace(NewWorkspace { name: "mbw".into() })
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

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    let auth = AuthContext::from_session(
        owner.id,
        ws.id,
        vec![WORKSPACE_WRITE.to_string(), WORKSPACE_READ.to_string()],
    );
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // A child's result block, created empty.
    server
        .call_tool(
            &auth,
            "create_memory_block",
            &json!({ "label": "child.result" }),
        )
        .await
        .unwrap();

    // A parent waits; the child rewrites the block → the wait wakes with the
    // fresh value (proving the MemoryBlockUpdated emit + the wait match).
    let wait_args = json!({ "label": "child.result", "timeout_ms": 5000 });
    let set_args = json!({ "label": "child.result", "value": "done" });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_memory_block", &wait_args),
        async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            server
                .call_tool(&auth, "set_memory_block_value", &set_args)
                .await
                .unwrap();
        }
    );
    let woke = content(out.unwrap());
    assert_eq!(woke["value"], "done");
    assert_eq!(woke["label"], "child.result");

    // An update to a DIFFERENT block does not wake this waiter → it times out.
    server
        .call_tool(&auth, "create_memory_block", &json!({ "label": "other" }))
        .await
        .unwrap();
    let wait_args = json!({ "label": "child.result", "timeout_ms": 400 });
    let other_args = json!({ "label": "other", "value": "x" });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_memory_block", &wait_args),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            server
                .call_tool(&auth, "set_memory_block_value", &other_args)
                .await
                .unwrap();
        }
    );
    assert!(
        content(out.unwrap()).is_null(),
        "an update to another block must not wake this waiter"
    );
}

#[tokio::test]
async fn glossary_tools_set_get_and_list() {
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
        .create_workspace(NewWorkspace { name: "gl".into() })
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

    let set = content(
        server
            .call_tool(
                &auth,
                "set_glossary_term",
                &json!({ "term": "LSN", "definition": "log sequence number", "aliases": ["wal position"] }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(set["term"], json!("LSN"));
    assert_eq!(set["definition"], json!("log sequence number"));

    let got = content(
        server
            .call_tool(&auth, "get_glossary_term", &json!({ "term": "LSN" }))
            .await
            .unwrap(),
    );
    assert_eq!(got["aliases"], json!(["wal position"]));

    // Unknown term -> null.
    let miss = content(
        server
            .call_tool(&auth, "get_glossary_term", &json!({ "term": "nope" }))
            .await
            .unwrap(),
    );
    assert!(miss.is_null());

    let list = content(
        server
            .call_tool(&auth, "list_glossary_terms", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["term"], json!("LSN"));
}

#[tokio::test]
async fn skill_tools_declare_and_list() {
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
        .create_workspace(NewWorkspace { name: "sk".into() })
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
    );
    let auth = AuthContext::bypass();
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    server
        .call_tool(
            &auth,
            "add_member_skill",
            &json!({ "member_id": member.id.0, "skill": "rust" }),
        )
        .await
        .unwrap();
    let skills = content(
        server
            .call_tool(
                &auth,
                "list_member_skills",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(skills.as_array().unwrap().len(), 1);
    assert_eq!(skills[0]["skill"], json!("rust"));

    server
        .call_tool(
            &auth,
            "add_thread_required_skill",
            &json!({ "thread_id": thread.id.0, "skill": "code-review" }),
        )
        .await
        .unwrap();
    let reqs = content(
        server
            .call_tool(
                &auth,
                "list_thread_required_skills",
                &json!({ "thread_id": thread.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(reqs.as_array().unwrap().len(), 1);
    assert_eq!(reqs[0]["skill"], json!("code-review"));
}
