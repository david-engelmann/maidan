//! Who may act: personal state, delegated tokens, private channels,
//! tenant isolation, attribution, secrets and freezes.

use super::*;

/// D-5: a member tool must not mutate personal state that is not the
/// caller's — and must not reach outside the caller's workspace at all.
///
/// Individual handlers do not consistently need `auth`, so the shared
/// pre-dispatch identity gate owns this invariant. `set_member_email` is
/// the sharp example: the address it writes is where that member's digest
/// mail goes. Member skills prove the guard covers tools outside
/// `tools/member.rs` too.
#[tokio::test]
async fn a_member_tool_cannot_mutate_another_members_personal_state() {
    use maidan_auth::capability::{MEMBER_SELF, MESSAGE_POST, WORKSPACE_READ, WORKSPACE_WRITE};
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

    let mk_ws = |name: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_workspace(NewWorkspace { name: name.into() })
                .await
                .unwrap()
        }
    };
    let ws_a = mk_ws("tenant-a").await;
    let ws_b = mk_ws("tenant-b").await;

    let mk_member = |ws: maidan_types::WorkspaceId, handle: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
        }
    };
    let attacker = mk_member(ws_a.id, "attacker").await;
    let same_tenant_victim = mk_member(ws_a.id, "colleague").await;
    let other_tenant_victim = mk_member(ws_b.id, "stranger").await;

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );

    // An ordinary least-privilege token: read the workspace, post messages.
    // Nothing about it suggests authority over anyone else's mailbox.
    let auth = AuthContext::from_token(
        maidan_types::ApiTokenId(uuid::Uuid::new_v4()),
        attacker.id,
        ws_a.id,
        vec![
            WORKSPACE_READ.to_string(),
            WORKSPACE_WRITE.to_string(),
            MEMBER_SELF.to_string(),
            MESSAGE_POST.to_string(),
        ],
    );

    // Own state: allowed.
    server
        .call_tool(
            &auth,
            "set_member_email",
            &json!({ "member_id": attacker.id.0, "email": "me@example.com" }),
        )
        .await
        .expect("a member may set their own delivery address");
    server
        .call_tool(
            &auth,
            "add_member_skill",
            &json!({ "member_id": attacker.id.0, "skill": "rust" }),
        )
        .await
        .expect("a member may declare their own routing skill");

    // A colleague's state: refused.
    let err = server
        .call_tool(
            &auth,
            "set_member_email",
            &json!({ "member_id": same_tenant_victim.id.0, "email": "attacker@evil.test" }),
        )
        .await
        .expect_err("a token must not redirect another member's mail");
    assert!(
        matches!(err, McpError::Forbidden(_)),
        "expected Forbidden, got {err:?}"
    );
    for tool in ["add_member_skill", "list_member_skills"] {
        let args = if tool == "add_member_skill" {
            json!({ "member_id": same_tenant_victim.id.0, "skill": "rust" })
        } else {
            json!({ "member_id": same_tenant_victim.id.0 })
        };
        let err = server
            .call_tool(&auth, tool, &args)
            .await
            .expect_err("a token must not inspect or rewrite another member's skills");
        assert!(matches!(err, McpError::Forbidden(_)), "{tool}: {err:?}");
    }

    // Another tenant's state: refused, and indistinguishably so — a
    // different error here would confirm the member exists.
    let err = server
        .call_tool(
            &auth,
            "set_member_email",
            &json!({ "member_id": other_tenant_victim.id.0, "email": "attacker@evil.test" }),
        )
        .await
        .expect_err("a token must not reach outside its workspace");
    assert!(
        matches!(err, McpError::Forbidden(_)),
        "expected Forbidden, got {err:?}"
    );

    // The victims' addresses are untouched.
    for victim in [same_tenant_victim.id, other_tenant_victim.id] {
        assert!(
            store.get_member_email(victim).await.unwrap().is_none(),
            "a refused call must not have written"
        );
    }
}

/// An orchestrator acting for one of its agents holds a delegated token for
/// that agent. The token *is* the agent: it may touch the agent's personal
/// state, not a stranger's in another tenant — and not the orchestrator's
/// own either, because a delegated token is its subject, not both parties.
#[tokio::test]
async fn a_delegated_token_acts_as_its_subject_and_no_one_else() {
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

    let mk_ws = |name: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_workspace(NewWorkspace { name: name.into() })
                .await
                .unwrap()
        }
    };
    let ws_a = mk_ws("orchestrated").await;
    let ws_b = mk_ws("other-tenant").await;
    let mk_member = |ws: maidan_types::WorkspaceId, handle: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
        }
    };
    let orchestrator = mk_member(ws_a.id, "orchestrator").await;
    let worker = mk_member(ws_a.id, "worker").await;
    let stranger = mk_member(ws_b.id, "stranger").await;

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_delegated_token(
        maidan_types::ApiTokenId(uuid::Uuid::new_v4()),
        orchestrator.id,
        worker.id,
        ws_a.id,
        maidan_types::DelegationGrantId(uuid::Uuid::new_v4()),
        vec![WORKSPACE_READ.to_string(), MEMBER_SELF.to_string()],
    );

    server
        .call_tool(
            &auth,
            "set_member_email",
            &json!({ "member_id": worker.id.0, "email": "worker@example.com" }),
        )
        .await
        .expect("a delegated token acts as its subject");

    for (target, why) in [
        (stranger.id, "a member of another tenant"),
        (
            orchestrator.id,
            "the delegate itself — the token is the subject, not both",
        ),
    ] {
        let err = server
            .call_tool(
                &auth,
                "set_member_email",
                &json!({ "member_id": target.0, "email": "attacker@evil.test" }),
            )
            .await
            .expect_err(why);
        assert!(
            matches!(err, McpError::Forbidden(_)),
            "{why}: expected Forbidden, got {err:?}"
        );
        assert!(
            store.get_member_email(target).await.unwrap().is_none(),
            "{why}: a refused call must not have written"
        );
    }
}

/// The MCP twins of grant creation and exchange carry the same bounds as
/// REST: a grant cannot lend authority, and a borrowed token cannot exchange.
#[tokio::test]
async fn mcp_delegation_is_bounded_like_rest() {
    use maidan_auth::capability::{TOKEN_ADMIN, WORKSPACE_READ};
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
    let mut ids = Vec::new();
    for handle in ["admin", "orchestrator", "worker", "third"] {
        ids.push(
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
    let (admin, orchestrator, worker, third) = (ids[0], ids[1], ids[2], ids[3]);
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );

    let admin_auth = AuthContext::from_token(
        ApiTokenId(uuid::Uuid::new_v4()),
        admin,
        ws.id,
        vec![TOKEN_ADMIN.to_string()],
    );
    let err = server
        .call_tool(
            &admin_auth,
            "create_delegation_grant",
            &json!({
                "workspace_id": ws.id.0,
                "subject_id": worker.0,
                "delegate_id": orchestrator.0,
                "capabilities": [TOKEN_ADMIN],
                "purpose": "lend authority",
                "expires_at": chrono::Utc::now() + chrono::Duration::hours(1),
            }),
        )
        .await
        .expect_err("a grant must not lend authority over MCP either");
    assert!(
        matches!(err, McpError::InvalidParams(_)),
        "expected InvalidParams, got {err:?}"
    );

    // The orchestrator's own grant, presented from a token borrowed as the
    // worker: the delegate check alone would pass it, so this is the case
    // only the one-hop rule stops.
    let own = store
        .create_delegation_grant(NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: third,
            delegate_id: orchestrator,
            capabilities: vec![WORKSPACE_READ.into()],
            purpose: "second grant".into(),
            authorized_by: admin,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();
    let borrowed = AuthContext::from_delegated_token(
        ApiTokenId(uuid::Uuid::new_v4()),
        orchestrator,
        worker,
        ws.id,
        DelegationGrantId(uuid::Uuid::new_v4()),
        vec![WORKSPACE_READ.to_string()],
    );
    let err = server
        .call_tool(
            &borrowed,
            "delegate_token",
            &json!({ "grant_id": own.id.0 }),
        )
        .await
        .expect_err("a borrowed token must not exchange over MCP either");
    assert!(
        matches!(err, McpError::Forbidden(_)),
        "expected Forbidden, got {err:?}"
    );
}

/// MCP over stdio never passes through the HTTP auth middleware, so the
/// scope `tools_call` sets is the only thing attributing its writes. This
/// calls the server directly, as stdio does.
#[tokio::test]
async fn an_mcp_tool_call_attributes_what_it_writes() {
    use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ};
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
    let mut ids = Vec::new();
    for handle in ["orchestrator", "worker"] {
        ids.push(
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
    let (orchestrator, worker) = (ids[0], ids[1]);
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
    let grant = DelegationGrantId(uuid::Uuid::new_v4());
    let borrowed = AuthContext::from_delegated_token(
        ApiTokenId(uuid::Uuid::new_v4()),
        orchestrator,
        worker,
        ws.id,
        grant,
        vec![WORKSPACE_READ.to_string(), MESSAGE_POST.to_string()],
    );
    server
        .call_tool(
            &borrowed,
            "post_message",
            &json!({ "thread_id": thread.id.0, "body": "via stdio" }),
        )
        .await
        .expect("post as the worker");

    let posted = store
        .list_events_after(ws.id, 0, 100)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.kind == EventKind::MessagePosted)
        .expect("a MessagePosted event");
    assert_eq!(
        posted.attribution(),
        Some(Attribution {
            actor_id: orchestrator,
            subject_id: worker,
            grant_id: Some(grant),
        })
    );
}

#[tokio::test]
async fn mcp_denies_non_members_in_private_channels() {
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
            name: "rbac".into(),
        })
        .await
        .unwrap();
    let mk = |handle: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap()
        }
    };
    let alice = mk("alice").await;
    let bob = mk("bob").await;
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "secret".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    store
        .add_channel_member(ch.id, alice.id, ChannelMemberRole::Admin)
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
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

    let caps = vec![
        maidan_auth::capability::WORKSPACE_READ.to_string(),
        maidan_auth::capability::MESSAGE_POST.to_string(),
    ];
    let bob_auth = AuthContext::from_session(bob.id, ws.id, caps.clone());
    let alice_auth = AuthContext::from_session(alice.id, ws.id, caps);
    let args = json!({ "thread_id": thread.id.0 });

    // Non-member Bob is denied; member Alice is allowed.
    let err = server
        .call_tool(&bob_auth, "list_messages", &args)
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Forbidden(_)));
    server
        .call_tool(&alice_auth, "list_messages", &args)
        .await
        .unwrap();

    // Aggregate reads: the private channel is filtered out of list_channels
    // + get_workspace_context for Bob, present for Alice.
    let unwrap_content = |v: Value| -> Value {
        let text = v["content"][0]["text"].as_str().unwrap().to_string();
        serde_json::from_str(&text).unwrap()
    };
    let ws_args = json!({ "workspace_id": ws.id.0 });

    let bob_channels = unwrap_content(
        server
            .call_tool(&bob_auth, "list_channels", &ws_args)
            .await
            .unwrap(),
    );
    assert!(
        !bob_channels
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == json!(ch.id.0)),
        "bob must not see the private channel"
    );
    let alice_channels = unwrap_content(
        server
            .call_tool(&alice_auth, "list_channels", &ws_args)
            .await
            .unwrap(),
    );
    assert!(
        alice_channels
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == json!(ch.id.0)),
        "alice sees the private channel"
    );

    let bob_ctx = unwrap_content(
        server
            .call_tool(&bob_auth, "get_workspace_context", &ws_args)
            .await
            .unwrap(),
    );
    assert!(
        bob_ctx["threads"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["channel_id"] != json!(ch.id.0)),
        "bob's workspace context excludes the private channel's threads"
    );
}

#[tokio::test]
async fn secret_egress_host_tools_need_admin_and_read_and_are_audited() {
    use maidan_auth::capability::{SECRET_ADMIN, SECRET_READ};

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
        .create_workspace(NewWorkspace { name: "sec".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool.clone())),
        Arc::new(HashV1Provider),
    );
    server.set_secret_egress_ceiling(vec!["hooks.example.com".into()]);
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let admin_only = AuthContext::from_session(member.id, ws.id, vec![SECRET_ADMIN.into()]);
    let admin = AuthContext::from_session(
        member.id,
        ws.id,
        vec![SECRET_ADMIN.into(), SECRET_READ.into()],
    );
    let allow = json!({ "host": "Hooks.Example.com" });

    // Listing a host hands it the values, so rotating secrets is not enough.
    let err = server
        .call_tool(&admin_only, "allow_secret_egress_host", &allow)
        .await
        .unwrap_err();
    assert!(
        matches!(err, McpError::Forbidden(ref m) if m.contains("secret:read")),
        "{err:?}"
    );
    assert!(store
        .list_secret_egress_hosts(ws.id)
        .await
        .unwrap()
        .is_empty());

    let entry = content(
        server
            .call_tool(&admin, "allow_secret_egress_host", &allow)
            .await
            .unwrap(),
    );
    assert_eq!(entry["host"], json!("hooks.example.com"));
    for refused in ["https://hooks.example.com", "other.example.com"] {
        let err = server
            .call_tool(
                &admin,
                "allow_secret_egress_host",
                &json!({ "host": refused }),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, McpError::InvalidParams(_)),
            "{refused}: {err:?}"
        );
    }

    let listed = content(
        server
            .call_tool(&admin_only, "list_secret_egress_hosts", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(listed.as_array().unwrap().len(), 1);

    server
        .call_tool(
            &admin_only,
            "revoke_secret_egress_host",
            &json!({ "host": "hooks.example.com" }),
        )
        .await
        .unwrap();
    assert!(store
        .list_secret_egress_hosts(ws.id)
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        server
            .call_tool(
                &admin_only,
                "revoke_secret_egress_host",
                &json!({ "host": "hooks.example.com" }),
            )
            .await,
        Err(McpError::NotFound)
    ));

    let audit = store.list_audit(20).await.unwrap();
    for action in ["secret_egress_host.allow", "secret_egress_host.revoke"] {
        assert!(
            audit
                .iter()
                .any(|row| row.action == action && row.metadata["host"] == "hooks.example.com"),
            "{action} unrecorded"
        );
    }
}

#[tokio::test]
async fn secret_tools_list_and_resolve() {
    use maidan_auth::capability::SECRET_READ;
    use maidan_types::NewSecret;

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
        .create_workspace(NewWorkspace { name: "sec".into() })
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

    // Seed a secret whose ciphertext is encrypted with the same key the server
    // will hold (the route layer does this in production).
    let key: Arc<[u8; 32]> = Arc::new([9u8; 32]);
    let ciphertext = maidan_auth::encrypt_peer_secret("s3cr3t", &key).unwrap();
    store
        .create_secret(NewSecret {
            workspace_id: ws.id,
            name: "api-key".into(),
            value_ciphertext: ciphertext,
            created_by: member.id,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool.clone())),
        Arc::new(HashV1Provider),
    );
    server.set_encryption_key(key);
    let auth = AuthContext::from_session(member.id, ws.id, vec![SECRET_READ.to_string()]);
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // List returns metadata only.
    let list = content(
        server
            .call_tool(&auth, "list_secrets", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], json!("api-key"));
    assert!(list[0].get("value").is_none());

    // Resolve decrypts the value.
    let resolved = content(
        server
            .call_tool(&auth, "resolve_secret", &json!({ "name": "api-key" }))
            .await
            .unwrap(),
    );
    assert_eq!(resolved["value"], json!("s3cr3t"));

    // An unknown name is a not-found error.
    assert!(server
        .call_tool(&auth, "resolve_secret", &json!({ "name": "ghost" }))
        .await
        .is_err());

    // A resolve is recorded, and the value is withheld when it cannot be.
    assert!(store
        .list_audit(10)
        .await
        .unwrap()
        .iter()
        .any(|row| row.action == "secret.resolve" && row.metadata["name"] == "api-key"));
    sqlx::query(
        "CREATE TRIGGER audit_down BEFORE INSERT ON maidan_audit
             BEGIN SELECT RAISE(ABORT, 'audit down'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(server
        .call_tool(&auth, "resolve_secret", &json!({ "name": "api-key" }))
        .await
        .is_err());
}

#[tokio::test]
async fn freeze_tools_freeze_unfreeze_and_list() {
    use futures::StreamExt;
    use maidan_auth::capability::TOKEN_ADMIN;
    use maidan_bus::{BusItem, EventBus, InMemoryBus};
    use maidan_types::NewThread;
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
        .create_workspace(NewWorkspace { name: "frz".into() })
        .await
        .unwrap();
    let op = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
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
    store.assign_thread(thread.id, agent.id).await.unwrap();
    let elsewhere = store
        .create_workspace(NewWorkspace {
            name: "other".into(),
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
    let kinds = [EventKind::MemberFrozen, EventKind::MemberUnfrozen];
    let mut here = bus
        .subscribe(EventFilter::workspace(ws.id).with_kinds(kinds))
        .await
        .unwrap();
    let mut there = bus
        .subscribe(EventFilter::workspace(elsewhere.id).with_kinds(kinds))
        .await
        .unwrap();
    async fn next(stream: &mut maidan_bus::EventStream) -> Option<Event> {
        match tokio::time::timeout(Duration::from_millis(500), stream.next()).await {
            Ok(Some(BusItem::Event(envelope))) => Some(envelope.event),
            _ => None,
        }
    }
    let auth = AuthContext::from_session(op.id, ws.id, vec![TOKEN_ADMIN.to_string()]);
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Freeze drops the agent's lease.
    let frozen = content(
        server
            .call_tool(&auth, "freeze_member", &json!({ "member_id": agent.id.0 }))
            .await
            .unwrap(),
    );
    assert_eq!(frozen["released"], json!(1));
    assert_eq!(store.get_thread(thread.id).await.unwrap().assignee_id, None);
    match next(&mut here).await {
        Some(Event::MemberFrozen {
            member_id,
            frozen_by,
            released,
            ..
        }) => {
            assert_eq!((member_id, frozen_by, released), (agent.id, op.id, 1));
        }
        other => panic!("expected MemberFrozen, got {other:?}"),
    }

    // List shows the frozen agent.
    let list = content(
        server
            .call_tool(&auth, "list_frozen_members", &json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Unfreeze lifts it.
    let un = content(
        server
            .call_tool(
                &auth,
                "unfreeze_member",
                &json!({ "member_id": agent.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(un["unfrozen"], json!(true));
    assert!(!store.is_member_frozen(agent.id).await.unwrap());
    assert!(matches!(
        next(&mut here).await,
        Some(Event::MemberUnfrozen { member_id, unfrozen_by, .. })
            if member_id == agent.id && unfrozen_by == op.id
    ));
    assert!(
        next(&mut there).await.is_none(),
        "another workspace's subscriber saw the freeze"
    );
}

#[tokio::test]
async fn mcp_artifact_tools_enforce_tenant_isolation() {
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
    let mk = |name: &str| {
        let store = store.clone();
        let name = name.to_string();
        async move {
            let ws = store
                .create_workspace(NewWorkspace { name: name.clone() })
                .await
                .unwrap();
            let m = store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: name,
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap();
            (ws.id, m.id)
        }
    };
    let (ws_a, member_a) = mk("a").await;
    let (ws_b, member_b) = mk("b").await;
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool.clone())),
        Arc::new(HashV1Provider),
    );
    let caps = || vec![ARTIFACT_UPLOAD.to_string(), WORKSPACE_READ.to_string()];
    let auth_a = AuthContext::from_session(member_a, ws_a, caps());
    let auth_b = AuthContext::from_session(member_b, ws_b, caps());
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // Workspace A uploads an artifact (records A's access ref).
    let up = content(
        server
            .call_tool(
                &auth_a,
                "upload_artifact",
                &json!({ "kind": "attachment", "content_base64": "aGVsbG8=" }),
            )
            .await
            .unwrap(),
    );
    let sha = up["sha256"].as_str().unwrap().to_string();
    // ArtifactUpserted is deliberately global (the blob is deduplicated;
    // workspace access lives in the separate ref), so inspect the global
    // log rather than the workspace-filtered stream.
    let events = store.list_events_after_global(0, 100).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.kind == EventKind::ArtifactUpserted),
        "MCP upload_artifact must append ArtifactUpserted"
    );

    // A can read its own artifact metadata; B cannot (NotFound — no oracle).
    assert!(server
        .call_tool(&auth_a, "get_artifact_metadata", &json!({ "sha256": sha }))
        .await
        .is_ok());
    let denied = server
        .call_tool(&auth_b, "get_artifact_metadata", &json!({ "sha256": sha }))
        .await;
    assert!(
        matches!(denied, Err(McpError::NotFound)),
        "B must not read A's artifact metadata, got {denied:?}"
    );

    // Same isolation on the resources/read path (maidan://artifacts/{sha}).
    let req = |uri: String| JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: "resources/read".to_string(),
        params: json!({ "uri": uri }),
    };
    let uri = format!("maidan://artifacts/{sha}");
    let resp_b = server.handle(req(uri.clone()), &auth_b).await;
    assert!(
        resp_b.error.is_some() && resp_b.result.is_none(),
        "B must not read A's artifact via resources/read"
    );
    let resp_a = server.handle(req(uri.clone()), &auth_a).await;
    assert!(resp_a.result.is_some() && resp_a.error.is_none());
    let upper = format!("maidan://artifacts/{}", sha.to_ascii_uppercase());
    let resp_upper = server.handle(req(upper), &auth_a).await;
    assert!(
        resp_upper.result.is_some() && resp_upper.error.is_none(),
        "uppercase digest must resolve for the workspace that holds it, got {resp_upper:?}"
    );
}
