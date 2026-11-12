//! The handshake: protocol-version negotiation, `initialize`, `whoami` and
//! the content shapes a tool call accepts.

use super::*;

/// Typed content is enough: the server derives `body`. Omitting both is
/// rejected for a post and for an edit.
#[tokio::test]
async fn content_without_body_is_accepted_and_neither_is_refused() {
    let (server, thread_id, member_id) = mk_server().await;
    let thread = server.store.get_thread(thread_id).await.unwrap();
    let channel = server.store.get_channel(thread.channel_id).await.unwrap();
    let auth = AuthContext::from_session(
        member_id,
        channel.workspace_id,
        vec![maidan_auth::capability::MESSAGE_POST.to_string()],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let posted = unwrap_content(
        server
            .call_tool(
                &auth,
                "post_message",
                &json!({
                    "thread_id": thread_id.0,
                    "content": [{"type": "text", "text": "from blocks"}],
                }),
            )
            .await
            .expect("content without body is a valid post"),
    );
    assert_eq!(posted["body"], "from blocks");

    let missing = server
        .call_tool(&auth, "post_message", &json!({ "thread_id": thread_id.0 }))
        .await;
    assert!(
        matches!(&missing, Err(McpError::InvalidParams(m)) if m.contains("body")),
        "omitting body and content must fail, got {missing:?}"
    );

    let edited = unwrap_content(
        server
            .call_tool(
                &auth,
                "edit_message",
                &json!({
                    "message_id": posted["id"],
                    "content": [{"type": "code", "code": "1 + 1"}],
                }),
            )
            .await
            .expect("content without body is a valid edit"),
    );
    assert_eq!(edited["body"], "```\n1 + 1\n```");

    let missing_edit = server
        .call_tool(
            &auth,
            "edit_message",
            &json!({ "message_id": posted["id"] }),
        )
        .await;
    assert!(
        matches!(&missing_edit, Err(McpError::InvalidParams(m)) if m.contains("body")),
        "omitting body and content must fail an edit, got {missing_edit:?}"
    );
}

#[test]
fn protocol_version_negotiation_follows_the_spec_rule() {
    // Supported requested version is echoed; unsupported/absent fall back to preferred.
    assert_eq!(negotiate_protocol_version(Some("2024-11-05")), "2024-11-05");
    // 2026-07-28 is negotiable: a current client that requests it gets it (J3.1).
    assert_eq!(negotiate_protocol_version(Some("2026-07-28")), "2026-07-28");
    assert_eq!(
        negotiate_protocol_version(Some("1999-01-01")),
        preferred_protocol_version()
    );
    assert_eq!(
        negotiate_protocol_version(None),
        preferred_protocol_version()
    );
    assert!(is_supported_protocol_version("2024-11-05"));
    assert!(is_supported_protocol_version("2026-07-28"));
    assert!(!is_supported_protocol_version("1999-01-01"));
    // The current default is 2026-07-28 (the full revision landed, J3.5); an
    // older client that explicitly requests 2024-11-05 still gets it.
    assert_eq!(preferred_protocol_version(), "2026-07-28");
}

#[tokio::test]
async fn whoami_returns_identity_and_initialize_carries_instructions() {
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
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
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
    let auth = AuthContext::from_token(
        maidan_types::ApiTokenId(uuid::Uuid::new_v4()),
        member.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), MESSAGE_POST.to_string()],
    );
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let me = content(server.call_tool(&auth, "whoami", &json!({})).await.unwrap());
    assert_eq!(me["member_id"], json!(member.id.0));
    assert_eq!(me["workspace_id"], json!(ws.id.0));
    assert_eq!(me["is_bearer"], json!(true));
    let caps = me["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|c| c == "workspace:read"));

    // initialize carries the spec `instructions` cold-start guide.
    let init = server
        .handle(
            JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(1)),
                method: "initialize".to_string(),
                params: json!({ "protocolVersion": "2026-07-28" }),
            },
            &auth,
        )
        .await;
    let instructions = init.result.unwrap()["instructions"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(instructions.contains("whoami"));
    assert!(instructions.contains("claim_next_thread"));
}

#[tokio::test]
async fn initialize_echoes_negotiated_version_and_the_initialized_notification_is_accepted() {
    let (server, _thread, member) = mk_server().await;
    let ws = server.store.get_member(member).await.unwrap().workspace_id;
    let auth = admin_auth(member, ws);
    let init = server
        .handle(
            request(1, "initialize", json!({ "protocolVersion": "2024-11-05" })),
            &auth,
        )
        .await;
    assert_eq!(init.result.unwrap()["protocolVersion"], "2024-11-05");
    // The post-init handshake notification is dispatched without error.
    let ack = server
        .handle(request(2, "notifications/initialized", json!({})), &auth)
        .await;
    assert!(ack.error.is_none());
}

#[tokio::test]
async fn initialize_reports_the_configured_server_version() {
    // The embedder's release version, threaded through with_server_version:
    // without it, initialize reports this crate's version (0.0.0).
    let (server, _thread, member) = mk_server().await;
    let server = server.with_server_version("413.0.0-test");
    let ws = server.store.get_member(member).await.unwrap().workspace_id;
    let auth = admin_auth(member, ws);
    let init = server
        .handle(
            request(1, "initialize", json!({ "protocolVersion": "2026-07-28" })),
            &auth,
        )
        .await;
    let result = init.result.unwrap();
    assert_eq!(result["serverInfo"]["name"], "maidan");
    assert_eq!(result["serverInfo"]["version"], "413.0.0-test");
}

#[tokio::test]
async fn initialize_negotiates_the_2026_revision_when_requested() {
    // A current (2026-07-28) client's initialize is echoed that revision (J3.1);
    // 2026 clients need not call initialize, but when they do it must negotiate.
    let (server, _thread, member) = mk_server().await;
    let ws = server.store.get_member(member).await.unwrap().workspace_id;
    let auth = admin_auth(member, ws);
    let init = server
        .handle(
            request(1, "initialize", json!({ "protocolVersion": "2026-07-28" })),
            &auth,
        )
        .await;
    assert_eq!(init.result.unwrap()["protocolVersion"], "2026-07-28");
}
