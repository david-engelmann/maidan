//! A member's own surface: mutes, inbox, notifications and their
//! preferences, delivery mode, email and follows.

use super::*;

/// The per-channel mute MCP tools (self-scoped).
#[tokio::test]
async fn channel_mute_tools_mute_and_unmute() {
    use maidan_auth::capability::{MEMBER_SELF, WORKSPACE_READ};
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
        .create_workspace(NewWorkspace { name: "cm".into() })
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
            name: "busy".into(),
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
    let auth = AuthContext::from_session(
        member.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), MEMBER_SELF.to_string()],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let muted = unwrap_content(
        server
            .call_tool(
                &auth,
                "mute_channel",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(muted["muted"], json!(true));
    assert!(store.is_channel_muted(member.id, channel.id).await.unwrap());
    let unmuted = unwrap_content(
        server
            .call_tool(
                &auth,
                "unmute_channel",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(unmuted["unmuted"], json!(true));
    let again = unwrap_content(
        server
            .call_tool(
                &auth,
                "unmute_channel",
                &json!({ "channel_id": channel.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(again["unmuted"], json!(false), "second unmute is a no-op");
}

/// The get_waiting_inbox tool composes a member's assigned non-terminal
/// threads + the workspace's pending gates.
#[tokio::test]
async fn waiting_inbox_tool_composes_assigned_and_gates() {
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
        .create_workspace(NewWorkspace { name: "wi".into() })
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
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store.assign_thread(thread.id, agent.id).await.unwrap();
    store
        .create_approval_gate(&maidan_types::NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: agent.id,
            prompt: "approve".into(),
            schema: None,
            risk: Default::default(),
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = admin_auth(agent.id, ws.id);
    let inbox: Value = serde_json::from_str(
        server
            .call_tool(
                &auth,
                "get_waiting_inbox",
                &json!({ "member_id": agent.id.0 }),
            )
            .await
            .unwrap()["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(inbox["total"], json!(2));
    let kinds: Vec<&str> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"assigned_thread"));
    assert!(kinds.contains(&"open_gate"));
}

#[tokio::test]
async fn notification_tools_list_count_mark_and_wait() {
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
        .create_workspace(NewWorkspace { name: "n".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "me".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
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
    let writer = store.clone();
    // Seed two notifications for the member.
    let mut ids = Vec::new();
    for log_id in 1..=2 {
        let n = store
            .create_notification(NewNotification {
                workspace_id: ws.id,
                member_id: member.id,
                kind: EventKind::MentionRecorded,
                source_log_id: log_id,
                channel_id: Some(channel.id),
                thread_id: Some(thread.id),
                message_id: None,
                actor_id: None,
            })
            .await
            .unwrap();
        ids.push(n.id);
    }

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    let auth = admin_auth(member.id, ws.id);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let mid = json!({ "member_id": member.id.0 });

    // Count + list see both.
    let count = unwrap_content(
        server
            .call_tool(&auth, "get_unread_count", &mid)
            .await
            .unwrap(),
    );
    assert_eq!(count["count"], json!(2));
    let list = unwrap_content(
        server
            .call_tool(&auth, "list_notifications", &mid)
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 2);

    // Mark one read → count drops, unread_only filters.
    let marked = unwrap_content(
        server
            .call_tool(
                &auth,
                "mark_notification_read",
                &json!({ "member_id": member.id.0, "notification_id": ids[0].0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(marked["marked"], json!(true));
    let count = unwrap_content(
        server
            .call_tool(&auth, "get_unread_count", &mid)
            .await
            .unwrap(),
    );
    assert_eq!(count["count"], json!(1));
    let unread = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_notifications",
                &json!({ "member_id": member.id.0, "unread_only": true }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(unread.as_array().unwrap().len(), 1);

    // Snooze: snoozing the remaining unread notification into the future
    // clears the badge.
    let snoozed = unwrap_content(
        server
            .call_tool(
                &auth,
                "snooze_notification",
                &json!({ "member_id": member.id.0, "notification_id": ids[1].0, "until": "2999-01-01T00:00:00Z" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(snoozed["snoozed"], json!(true));
    let count = unwrap_content(
        server
            .call_tool(&auth, "get_unread_count", &mid)
            .await
            .unwrap(),
    );
    assert_eq!(
        count["count"],
        json!(0),
        "a snoozed notification is off the badge"
    );

    // wait_for_notification wakes on the member's next notification row,
    // whatever its kind, and carries the thread's title and a reason.
    let wait_args = json!({ "member_id": member.id.0, "timeout_ms": 5000 });
    let next_log_id = writer.max_event_id().await.unwrap() + 1_000;
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_notification", &wait_args),
        async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            writer
                .create_notification(NewNotification {
                    workspace_id: ws.id,
                    member_id: member.id,
                    kind: EventKind::ClaimStalled,
                    source_log_id: next_log_id,
                    channel_id: Some(channel.id),
                    thread_id: Some(thread.id),
                    message_id: None,
                    actor_id: None,
                })
                .await
                .unwrap();
        }
    );
    let woke = unwrap_content(out.unwrap());
    assert_eq!(woke["kind"], json!("claim_stalled"));
    assert_eq!(woke["member_id"], json!(member.id.0));
    assert_eq!(woke["source_log_id"], json!(next_log_id));
    assert_eq!(woke["title"], json!("t"));

    // A row written before the wait began is returned when the caller
    // says where it last looked, and not otherwise.
    let caught_up = unwrap_content(
        server
            .call_tool(
                &auth,
                "wait_for_notification",
                &json!({ "member_id": member.id.0, "timeout_ms": 1, "since_log_id": next_log_id - 1 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(caught_up["source_log_id"], json!(next_log_id));
    let lapsed = unwrap_content(
        server
            .call_tool(
                &auth,
                "wait_for_notification",
                &json!({ "member_id": member.id.0, "timeout_ms": 1, "since_log_id": next_log_id }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(lapsed, Value::Null);
}

#[tokio::test]
async fn notification_pref_tools_set_and_list() {
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
    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = admin_auth(member.id, ws.id);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Set a mute.
    let pref = unwrap_content(
        server
            .call_tool(
                &auth,
                "set_notification_pref",
                &json!({ "member_id": member.id.0, "kind": "mention_recorded", "muted": true }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(pref["kind"], json!("mention_recorded"));
    assert_eq!(pref["muted"], json!(true));

    // List reflects it.
    let list = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_notification_prefs",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["muted"], json!(true));

    // An unknown kind is rejected.
    assert!(server
        .call_tool(
            &auth,
            "set_notification_pref",
            &json!({ "member_id": member.id.0, "kind": "not_a_kind", "muted": true }),
        )
        .await
        .is_err());
}

#[tokio::test]
async fn delivery_mode_tools_get_and_set() {
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
        .create_workspace(NewWorkspace { name: "d".into() })
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
    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = admin_auth(member.id, ws.id);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Default is immediate.
    let got = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_delivery_mode",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(got["mode"], json!("immediate"));

    // Switch to digest.
    let set = unwrap_content(
        server
            .call_tool(
                &auth,
                "set_delivery_mode",
                &json!({ "member_id": member.id.0, "mode": "digest" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(set["mode"], json!("digest"));

    // Get reflects it.
    let got = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_delivery_mode",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(got["mode"], json!("digest"));

    // An unknown mode is rejected.
    assert!(server
        .call_tool(
            &auth,
            "set_delivery_mode",
            &json!({ "member_id": member.id.0, "mode": "carrier-pigeon" }),
        )
        .await
        .is_err());
}

#[tokio::test]
async fn email_tools_set_get_delete() {
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
            handle: "m".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = admin_auth(member.id, ws.id);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Unset → null.
    let got = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_member_email",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert!(got.is_null());

    // Set, then get returns it.
    let set = unwrap_content(
        server
            .call_tool(
                &auth,
                "set_member_email",
                &json!({ "member_id": member.id.0, "email": "m@example.com" }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(set["email"], json!("m@example.com"));
    let got = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_member_email",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(got["email"], json!("m@example.com"));

    // A garbage address is rejected.
    assert!(server
        .call_tool(
            &auth,
            "set_member_email",
            &json!({ "member_id": member.id.0, "email": "nope" }),
        )
        .await
        .is_err());

    // Delete → {deleted:true}, then get is null again.
    let del = unwrap_content(
        server
            .call_tool(
                &auth,
                "delete_member_email",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(del["deleted"], json!(true));
    let got = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_member_email",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert!(got.is_null());
}

#[tokio::test]
async fn follow_tools_channel_thread_and_member() {
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
        .create_workspace(NewWorkspace { name: "f".into() })
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
    let colleague = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "colleague".into(),
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
    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = admin_auth(member.id, ws.id);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let args = json!({ "member_id": member.id.0, "channel_id": channel.id.0 });

    // Follow → list shows it → unfollow removes it.
    let followed = unwrap_content(
        server
            .call_tool(&auth, "follow_channel", &args)
            .await
            .unwrap(),
    );
    assert_eq!(followed["following"], json!(true));
    let list = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_channel_follows",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    let removed = unwrap_content(
        server
            .call_tool(&auth, "unfollow_channel", &args)
            .await
            .unwrap(),
    );
    assert_eq!(removed["removed"], json!(true));
    let empty = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_channel_follows",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert!(empty.as_array().unwrap().is_empty());

    let member_args = json!({ "member_id": member.id.0, "followed_member_id": colleague.id.0 });
    let followed = unwrap_content(
        server
            .call_tool(&auth, "follow_member", &member_args)
            .await
            .unwrap(),
    );
    assert_eq!(followed["following"], json!(true));
    let list = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_member_follows",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["followed_id"], json!(colleague.id.0));
    let occupancy = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_member_occupancy",
                &json!({ "member_id": colleague.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(occupancy["member_id"], json!(colleague.id.0));
    assert_eq!(occupancy["presence"], json!("offline"));
    assert_eq!(occupancy["assigned_threads"], json!([]));
    server
        .store
        .create_notification(NewNotification {
            workspace_id: ws.id,
            member_id: member.id,
            kind: EventKind::ApprovalRequested,
            source_log_id: 404,
            channel_id: Some(channel.id),
            thread_id: None,
            message_id: None,
            actor_id: Some(colleague.id),
        })
        .await
        .unwrap();
    let digest = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_manager_digest",
                &json!({ "member_id": member.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(digest["channels"][0]["channel_id"], json!(channel.id.0));
    assert_eq!(digest["channels"][0]["gates"], json!(1));
    let session = AuthContext::from_session(
        member.id,
        ws.id,
        vec![maidan_auth::capability::WORKSPACE_READ.to_string()],
    );
    let err = server
        .call_tool(
            &session,
            "get_manager_digest",
            &json!({ "member_id": colleague.id.0 }),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, crate::error::McpError::Forbidden(_)));
    let removed = unwrap_content(
        server
            .call_tool(&auth, "unfollow_member", &member_args)
            .await
            .unwrap(),
    );
    assert_eq!(removed["removed"], json!(true));
    let err = server
        .call_tool(
            &auth,
            "follow_member",
            &json!({ "member_id": member.id.0, "followed_member_id": member.id.0 }),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, crate::error::McpError::InvalidParams(_)));
}

#[tokio::test]
async fn inbox_tools_surface_a_members_mentions() {
    let (server, thread, member) = mk_server().await;
    let ws = server.store.get_member(member).await.unwrap().workspace_id;
    let auth = admin_auth(member, ws);

    // Post a message and record a mention of the member.
    let msg = server
        .store
        .post_message(NewMessage {
            thread_id: thread,
            author_id: member,
            body: "hey there".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();
    server.store.record_mention(msg.id, member).await.unwrap();

    let parse = |out: Value| -> Value {
        serde_json::from_str(out["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // list_mentions returns the mention.
    let out = server
        .call_tool(&auth, "list_mentions", &json!({ "member_id": member.0 }))
        .await
        .unwrap();
    assert_eq!(parse(out).as_array().unwrap().len(), 1);

    // get_inbox surfaces it too (as an object with the inbox shape).
    let out = server
        .call_tool(&auth, "get_inbox", &json!({ "member_id": member.0 }))
        .await
        .unwrap();
    assert!(parse(out).is_object());

    // mark_inbox_read advances the cursor and returns the inbox.
    let out = server
        .call_tool(
            &auth,
            "mark_inbox_read",
            &json!({ "member_id": member.0, "read_through": chrono::Utc::now().to_rfc3339() }),
        )
        .await
        .unwrap();
    assert_eq!(out["isError"], json!(false));
}
