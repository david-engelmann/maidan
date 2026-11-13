//! Domain events a tool call publishes, and slash-command dispatch.

use super::*;

/// A post the `max_tools` axis refuses comes back as a client error AND is
/// recorded as `ThreadSpawnDenied` naming the author.
#[tokio::test]
async fn a_refused_tool_post_publishes_thread_spawn_denied() {
    use std::time::Duration;

    use futures::StreamExt;
    use maidan_bus::{BusItem, InMemoryBus};

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
        .create_workspace(NewWorkspace { name: "sd".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "runaway".into(),
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
    // No tool calls at all on this thread.
    store
        .set_spawn_budget(ws.id, None, None, Some(0))
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
        .subscribe(EventFilter::all().with_kinds([EventKind::ThreadSpawnDenied]))
        .await
        .unwrap();

    let auth = AuthContext::from_session(
        member.id,
        ws.id,
        vec![
            maidan_auth::capability::MESSAGE_POST.to_string(),
            maidan_auth::capability::WORKSPACE_READ.to_string(),
        ],
    );
    let post = |content: Value| {
        let args = json!({
            "thread_id": thread.id.0,
            "body": "work",
            "content": content,
        });
        let auth = auth.clone();
        let server = &server;
        async move { server.call_tool(&auth, "post_message", &args).await }
    };

    // A plain post is unaffected by a tool budget.
    post(Value::Null).await.expect("a plain post still posts");

    let denied = post(json!([{
        "type": "tool_use", "id": "t1", "name": "run", "input": {}
    }]))
    .await;
    assert!(
        matches!(&denied, Err(McpError::InvalidParams(m)) if m.contains("tool")),
        "a tool-use post past max_tools=0 must be a client error, got {denied:?}"
    );

    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("timeout waiting for ThreadSpawnDenied")
        .expect("stream ended without event");
    let BusItem::Event(envelope) = event else {
        panic!("expected an event, got lag or end");
    };
    match envelope.event {
        Event::ThreadSpawnDenied {
            thread_id,
            member_id,
            axis,
            limit,
            observed,
            ..
        } => {
            assert_eq!(thread_id, thread.id);
            assert_eq!(member_id, Some(member.id), "the author is the actor");
            assert_eq!(axis, "tools");
            assert_eq!(limit, 0);
            assert_eq!(observed, 0);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn mcp_edit_message_appends_messageedited_event() {
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
        .create_workspace(NewWorkspace { name: "e".into() })
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
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "v1".into(),
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
        vec![
            maidan_auth::capability::MESSAGE_POST.to_string(),
            maidan_auth::capability::WORKSPACE_READ.to_string(),
        ],
    );

    server
        .call_tool(
            &auth,
            "edit_message",
            &json!({ "message_id": msg.id.0, "body": "v2" }),
        )
        .await
        .unwrap();

    // The edit now appends a MessageEdited event — so as-of replay sees "v2"
    // and the indexer reindexes (previously the MCP edit was event-less).
    let events = store
        .list_thread_events_through(thread.id, i64::MAX)
        .await
        .unwrap();
    assert!(
        events.iter().any(|e| e.kind == EventKind::MessageEdited),
        "MCP edit_message must append a MessageEdited event, got {:?}",
        events.iter().map(|e| e.kind).collect::<Vec<_>>()
    );
    // And the reconstructed as-of message shows the edited body.
    let msgs = maidan_types::reconstruct_messages_through(&events);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].body, "v2");
}

#[tokio::test]
async fn mcp_write_tools_emit_domain_events() {
    use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ, WORKSPACE_WRITE};
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
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let bob = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bob".into(),
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
    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: alice.id,
            body: "seed".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();

    // A bus so publish_event-based events (MessagePosted / MentionRecorded) log.
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    let auth = AuthContext::from_session(
        alice.id,
        ws.id,
        vec![
            MESSAGE_POST.to_string(),
            WORKSPACE_WRITE.to_string(),
            WORKSPACE_READ.to_string(),
        ],
    );

    server
        .call_tool(
            &auth,
            "cast_vote",
            &json!({ "message_id": msg.id.0, "kind": "approve" }),
        )
        .await
        .unwrap();
    for kind in ["up", "upvote", "request-changes", "ship", "👍"] {
        let refused = server
            .call_tool(
                &auth,
                "cast_vote",
                &json!({ "message_id": msg.id.0, "kind": kind }),
            )
            .await;
        assert!(
            matches!(
                &refused,
                Err(crate::error::McpError::InvalidParams(message))
                    if message.contains("approve")
                        && message.contains("request_changes")
                        && message.contains("ack")
            ),
            "{kind} was not rejected as an unknown vote kind: {refused:?}"
        );
    }
    // A verdict replaces the caller's opposing one; a retract takes it back once.
    server
        .call_tool(
            &auth,
            "cast_vote",
            &json!({ "message_id": msg.id.0, "kind": "request_changes" }),
        )
        .await
        .unwrap();
    let held = store.list_votes_for_message(msg.id).await.unwrap();
    assert_eq!(held.len(), 1, "one verdict per member: {held:?}");
    assert_eq!(held[0].kind, VoteKind::RequestChanges);
    for removed in [true, false] {
        let out = server
            .call_tool(
                &auth,
                "retract_vote",
                &json!({ "message_id": msg.id.0, "kind": "request_changes" }),
            )
            .await
            .unwrap();
        let text = out["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap()["removed"],
            removed
        );
    }
    assert!(store
        .list_votes_for_message(msg.id)
        .await
        .unwrap()
        .is_empty());
    server
        .call_tool(
            &auth,
            "add_reaction",
            &json!({ "message_id": msg.id.0, "emoji": "ship" }),
        )
        .await
        .unwrap();
    server
        .call_tool(
            &auth,
            "remove_reaction",
            &json!({ "message_id": msg.id.0, "emoji": "ship" }),
        )
        .await
        .unwrap();
    server
        .call_tool(
            &auth,
            "pin_message",
            &json!({
                "thread_id": thread.id.0,
                "message_id": msg.id.0
            }),
        )
        .await
        .unwrap();
    server
        .call_tool(
            &auth,
            "unpin_message",
            &json!({
                "thread_id": thread.id.0,
                "message_id": msg.id.0
            }),
        )
        .await
        .unwrap();
    server
        .call_tool(
            &auth,
            "add_reference",
            &json!({
                "src_kind": "message", "src_id": msg.id.0,
                "dst_kind": "thread", "dst_id": thread.id.0, "relation": "grounds"
            }),
        )
        .await
        .unwrap();
    server
        .call_tool(
            &auth,
            "post_message",
            &json!({ "thread_id": thread.id.0, "body": "ping @bob" }),
        )
        .await
        .unwrap();

    // ReferenceAdded is deliberately global, so use the global log to
    // prove the complete social/reference event surface in one pass.
    let events = store.list_events_after_global(0, 1000).await.unwrap();
    let kinds: Vec<EventKind> = events.iter().map(|e| e.kind).collect();
    for expected in [
        EventKind::VoteCast,
        EventKind::VoteRetracted,
        EventKind::ReactionAdded,
        EventKind::ReactionRemoved,
        EventKind::MessagePinned,
        EventKind::MessageUnpinned,
        EventKind::ReferenceAdded,
    ] {
        assert!(kinds.contains(&expected), "{expected:?}: {kinds:?}");
    }
    // The switch retracted approve, the retract removed request_changes,
    // and the second retract removed nothing.
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == EventKind::VoteRetracted)
            .count(),
        2,
        "{kinds:?}"
    );
    // …and an MCP post with an @mention now publishes MentionRecorded for the mentioned member.
    assert!(
        kinds.contains(&EventKind::MentionRecorded),
        "MentionRecorded: {kinds:?}"
    );
    let mentions = store.list_mentions_for_member(bob.id, 10).await.unwrap();
    assert_eq!(mentions.len(), 1, "bob was @mentioned");
}

#[tokio::test]
async fn mcp_post_runs_registered_slash_command_via_dispatcher() {
    use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ, WORKSPACE_WRITE};
    use maidan_bus::InMemoryBus;

    // Stands in for the server-side slash runtime: returns the slash
    // metadata the real `ServerSlashDispatcher` would produce.
    struct MockSlash;
    #[async_trait::async_trait]
    impl crate::slash_dispatch::SlashDispatcher for MockSlash {
        async fn dispatch(
            &self,
            _auth: &maidan_auth::AuthContext,
            parsed: &maidan_router::ParsedSlashCommand,
            _ws: WorkspaceId,
            _ch: ChannelId,
            _th: ThreadId,
            _author: MemberId,
            _msg: MessageId,
        ) -> serde_json::Value {
            json!({
                "slash_command": { "name": parsed.name, "args": parsed.args },
                "slash_response": { "ok": true, "handled": parsed.name }
            })
        }
    }

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
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
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
        .create_slash_command(NewSlashCommand {
            workspace_id: ws.id,
            name: "echo".into(),
            description: None,
            handler_kind: SlashHandlerKind::Http,
            handler_target: "https://example.test/hook".into(),
            secret_ciphertext: String::new(),
            created_by_token: None,
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
    server.set_slash_dispatcher(Arc::new(MockSlash));
    let auth = AuthContext::from_session(
        alice.id,
        ws.id,
        vec![
            MESSAGE_POST.to_string(),
            WORKSPACE_WRITE.to_string(),
            WORKSPACE_READ.to_string(),
        ],
    );

    // A `/echo` post runs the registered command → the message carries the
    // merged slash metadata (the parity with REST).
    server
        .call_tool(
            &auth,
            "post_message",
            &json!({ "thread_id": thread.id.0, "body": "/echo hi there" }),
        )
        .await
        .unwrap();
    let msgs = store.list_messages(thread.id, 10).await.unwrap();
    let slashed = msgs.iter().find(|m| m.body == "/echo hi there").unwrap();
    assert_eq!(slashed.metadata["slash_command"]["name"], "echo");
    assert_eq!(slashed.metadata["slash_command"]["args"], "hi there");
    assert_eq!(slashed.metadata["slash_response"]["ok"], true);

    // A plain post is untouched (no slash metadata).
    server
        .call_tool(
            &auth,
            "post_message",
            &json!({ "thread_id": thread.id.0, "body": "plain message" }),
        )
        .await
        .unwrap();
    let msgs = store.list_messages(thread.id, 10).await.unwrap();
    let plain = msgs.iter().find(|m| m.body == "plain message").unwrap();
    assert!(plain.metadata.get("slash_command").is_none());

    // An unregistered slash name is a literal post (no dispatch).
    server
        .call_tool(
            &auth,
            "post_message",
            &json!({ "thread_id": thread.id.0, "body": "/unknown x" }),
        )
        .await
        .unwrap();
    let msgs = store.list_messages(thread.id, 10).await.unwrap();
    let unknown = msgs.iter().find(|m| m.body == "/unknown x").unwrap();
    assert!(unknown.metadata.get("slash_command").is_none());
}
