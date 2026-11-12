//! The `wait_for_*` tools: each wakes on its event and never on another
//! tenant's or a private channel's.

use super::*;

#[tokio::test]
async fn wait_for_mention_returns_next_mention_and_filters_private() {
    use chrono::Utc;
    use maidan_auth::capability::WORKSPACE_READ;
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
        .create_workspace(NewWorkspace { name: "wm".into() })
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

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    // Not a member of the private channel — public is workspace-open.
    let auth = AuthContext::from_session(agent.id, ws.id, vec![WORKSPACE_READ.to_string()]);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let mention = |thread_id| Event::MentionRecorded {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        thread_id,
        message_id: MessageId(uuid::Uuid::new_v4()),
        member_id: agent.id,
    };

    // A mention in the public thread wakes the waiter. `join!` polls the
    // waiter first (it subscribes, then parks on the stream), then the
    // delayed publisher fires — no cross-task subscribe race.
    let live_args = json!({ "member_id": agent.id.0, "timeout_ms": 5000 });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_mention", &live_args),
        async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            server.publish_event(mention(pub_thread.id)).await;
        }
    );
    let got = unwrap_content(out.unwrap());
    assert_eq!(got["kind"], json!("mention_recorded"));
    assert_eq!(got["member_id"], json!(agent.id.0));
    assert_eq!(got["thread_id"], json!(pub_thread.id.0));

    // A mention in a private channel the agent can't access is skipped, so
    // the waiter times out to null despite a matching event being published.
    let filtered_args = json!({ "member_id": agent.id.0, "timeout_ms": 400 });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_mention", &filtered_args),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            server.publish_event(mention(secret_thread.id)).await;
        }
    );
    assert!(
        unwrap_content(out.unwrap()).is_null(),
        "a mention in an inaccessible private channel is filtered → timeout"
    );

    // A mention that lands BEFORE the waiter subscribes is still caught
    // when the caller passes its last-drain high-water as since_log_id — no
    // concurrent publish, the durable log replay finds it.
    let id = server.publish_event(mention(pub_thread.id)).await.unwrap();
    let lookback_args =
        json!({ "member_id": agent.id.0, "timeout_ms": 300, "since_log_id": id - 1 });
    let got = unwrap_content(
        server
            .call_tool(&auth, "wait_for_mention", &lookback_args)
            .await
            .unwrap(),
    );
    assert_eq!(got["kind"], json!("mention_recorded"));
    assert_eq!(got["thread_id"], json!(pub_thread.id.0));

    // since_log_id at the high-water (already seen) → nothing newer → null,
    // and it returns promptly rather than parking the full window.
    let seen_args = json!({ "member_id": agent.id.0, "timeout_ms": 300, "since_log_id": id });
    assert!(unwrap_content(
        server
            .call_tool(&auth, "wait_for_mention", &seen_args)
            .await
            .unwrap()
    )
    .is_null());

    // RBAC holds in the lookback path too: a pre-subscribe mention in a private
    // channel the agent can't access is filtered → null.
    let sid = server
        .publish_event(mention(secret_thread.id))
        .await
        .unwrap();
    let priv_args = json!({ "member_id": agent.id.0, "timeout_ms": 300, "since_log_id": sid - 1 });
    assert!(unwrap_content(
        server
            .call_tool(&auth, "wait_for_mention", &priv_args)
            .await
            .unwrap()
    )
    .is_null());
}

#[tokio::test]
async fn wait_for_ready_returns_next_ready_and_filters_private() {
    use chrono::Utc;
    use maidan_auth::capability::WORKSPACE_READ;
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
        .create_workspace(NewWorkspace { name: "wr".into() })
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

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    // Not a member of the private channel — public is workspace-open.
    let auth = AuthContext::from_session(agent.id, ws.id, vec![WORKSPACE_READ.to_string()]);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let ready = |thread: Thread| Event::ThreadReady {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        channel_id: thread.channel_id,
        thread_id: thread.id,
        thread,
    };

    // A ready task in the public thread wakes the workspace-wide waiter.
    let live_args = json!({ "timeout_ms": 5000 });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_ready", &live_args),
        async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            server.publish_event(ready(pub_thread.clone())).await;
        }
    );
    let got = unwrap_content(out.unwrap());
    assert_eq!(got["kind"], json!("thread_ready"));
    assert_eq!(got["thread_id"], json!(pub_thread.id.0));

    // A ready task in a private channel the agent can't access is skipped, so
    // the waiter times out to null despite a matching event being published.
    let filtered_args = json!({ "timeout_ms": 400 });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_ready", &filtered_args),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            server.publish_event(ready(secret_thread.clone())).await;
        }
    );
    assert!(
        unwrap_content(out.unwrap()).is_null(),
        "a ready task in an inaccessible private channel is filtered → timeout"
    );

    // Readiness signalled BEFORE the waiter subscribes is caught when the
    // caller passes since_log_id — no concurrent publish.
    let id = server
        .publish_event(ready(pub_thread.clone()))
        .await
        .unwrap();
    let lookback_args = json!({ "timeout_ms": 300, "since_log_id": id - 1 });
    let got = unwrap_content(
        server
            .call_tool(&auth, "wait_for_ready", &lookback_args)
            .await
            .unwrap(),
    );
    assert_eq!(got["kind"], json!("thread_ready"));
    assert_eq!(got["thread_id"], json!(pub_thread.id.0));

    // RBAC holds in the lookback path: a pre-subscribe ready in a private
    // channel the agent can't access is filtered → null.
    let sid = server
        .publish_event(ready(secret_thread.clone()))
        .await
        .unwrap();
    let priv_args = json!({ "timeout_ms": 300, "since_log_id": sid - 1 });
    assert!(unwrap_content(
        server
            .call_tool(&auth, "wait_for_ready", &priv_args)
            .await
            .unwrap()
    )
    .is_null());
}

#[tokio::test]
async fn wait_for_landed_returns_next_land_and_filters_private() {
    use chrono::Utc;
    use maidan_auth::capability::WORKSPACE_READ;
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
        .create_workspace(NewWorkspace { name: "wl".into() })
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

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    let auth = AuthContext::from_session(agent.id, ws.id, vec![WORKSPACE_READ.to_string()]);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let landed = |thread: Thread| Event::ThreadLanded {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        channel_id: thread.channel_id,
        thread_id: thread.id,
        repo: "o/r".into(),
        pr_number: 7,
        merged_by: Some("octocat".into()),
        merge_commit_sha: None,
        title: Some("add the widget".into()),
    };

    // A land in the public thread wakes the workspace-wide waiter.
    let live_args = json!({ "timeout_ms": 5000 });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_landed", &live_args),
        async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            server.publish_event(landed(pub_thread.clone())).await;
        }
    );
    let got = unwrap_content(out.unwrap());
    assert_eq!(got["kind"], json!("thread_landed"));
    assert_eq!(got["thread_id"], json!(pub_thread.id.0));
    assert_eq!(got["pr_number"], json!(7));

    // A land in a private channel the agent can't access is skipped → timeout.
    let filtered_args = json!({ "timeout_ms": 400 });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_landed", &filtered_args),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            server.publish_event(landed(secret_thread.clone())).await;
        }
    );
    assert!(
        unwrap_content(out.unwrap()).is_null(),
        "a land in an inaccessible private channel is filtered → timeout"
    );

    // A land emitted BEFORE the waiter subscribes is caught with
    // since_log_id — scoped to the specific thread.
    let id = server
        .publish_event(landed(pub_thread.clone()))
        .await
        .unwrap();
    let lookback_args =
        json!({ "thread_id": pub_thread.id.0, "timeout_ms": 300, "since_log_id": id - 1 });
    let got = unwrap_content(
        server
            .call_tool(&auth, "wait_for_landed", &lookback_args)
            .await
            .unwrap(),
    );
    assert_eq!(got["kind"], json!("thread_landed"));
    assert_eq!(got["thread_id"], json!(pub_thread.id.0));

    // A thread the agent can't read is refused before the wait parks.
    let refused = server
        .call_tool(
            &auth,
            "wait_for_landed",
            &json!({ "thread_id": secret_thread.id.0, "timeout_ms": 300 }),
        )
        .await;
    assert!(refused.is_err(), "an unreadable thread_id was waited on");
}

#[tokio::test]
async fn wait_for_claim_expired_returns_expiry_and_filters_private() {
    use chrono::Utc;
    use maidan_auth::capability::WORKSPACE_READ;
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
        .create_workspace(NewWorkspace { name: "ce".into() })
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

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    let auth = AuthContext::from_session(agent.id, ws.id, vec![WORKSPACE_READ.to_string()]);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let expired = |thread: Thread| Event::ClaimExpired {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        channel_id: thread.channel_id,
        thread_id: thread.id,
        member_id: agent.id,
        thread,
    };

    // An expiry in the public thread wakes the workspace-wide waiter, naming
    // the dead holder.
    let live_args = json!({ "timeout_ms": 5000 });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_claim_expired", &live_args),
        async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            server.publish_event(expired(pub_thread.clone())).await;
        }
    );
    let got = unwrap_content(out.unwrap());
    assert_eq!(got["kind"], json!("claim_expired"));
    assert_eq!(got["thread_id"], json!(pub_thread.id.0));
    assert_eq!(got["member_id"], json!(agent.id.0));

    // An expiry in a private channel the agent can't access is filtered → null.
    let filtered_args = json!({ "timeout_ms": 400 });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_claim_expired", &filtered_args),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            server.publish_event(expired(secret_thread.clone())).await;
        }
    );
    assert!(
        unwrap_content(out.unwrap()).is_null(),
        "an expiry in an inaccessible private channel is filtered → timeout"
    );

    // An expiry reclaimed BEFORE the waiter subscribes is caught via
    // since_log_id, and RBAC still filters a private one.
    let id = server
        .publish_event(expired(pub_thread.clone()))
        .await
        .unwrap();
    let lookback_args = json!({ "timeout_ms": 300, "since_log_id": id - 1 });
    let got = unwrap_content(
        server
            .call_tool(&auth, "wait_for_claim_expired", &lookback_args)
            .await
            .unwrap(),
    );
    assert_eq!(got["kind"], json!("claim_expired"));
    assert_eq!(got["thread_id"], json!(pub_thread.id.0));

    let sid = server
        .publish_event(expired(secret_thread.clone()))
        .await
        .unwrap();
    let priv_args = json!({ "timeout_ms": 300, "since_log_id": sid - 1 });
    assert!(unwrap_content(
        server
            .call_tool(&auth, "wait_for_claim_expired", &priv_args)
            .await
            .unwrap()
    )
    .is_null());
}

/// A workspace with a public and a private channel (the waiter is in
/// neither's member list; public is workspace-open), plus a second
/// workspace with its own thread, for the single-kind waits.
struct WaitFixture {
    server: McpServer,
    store: Arc<dyn Store>,
    auth: AuthContext,
    agent: Member,
    ws: WorkspaceId,
    public: Thread,
    public_sibling: Thread,
    secret: Thread,
    other_ws: WorkspaceId,
    other: Thread,
}

async fn wait_fixture(caps: &[&str]) -> WaitFixture {
    wait_fixture_on(caps, maidan_bus::InMemoryBus::new()).await
}

/// [`wait_fixture`] on a given bus, so a test can make it lag.
async fn wait_fixture_on(caps: &[&str], bus: maidan_bus::InMemoryBus) -> WaitFixture {
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
    let seed = |name: &'static str| {
        let store = store.clone();
        async move {
            let ws = store
                .create_workspace(NewWorkspace { name: name.into() })
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
            let channel = |name: &str, private: bool| NewChannel {
                workspace_id: ws.id,
                name: name.into(),
                topic: None,
                private,
            };
            let public = store.create_channel(channel("open", false)).await.unwrap();
            let secret = store.create_channel(channel("secret", true)).await.unwrap();
            let thread = |channel_id| NewThread {
                channel_id,
                parent_thread_id: None,
                title: Some("t".into()),
                description: None,
                created_by: None,
            };
            (
                ws.id,
                agent,
                store.create_thread(thread(public.id)).await.unwrap(),
                store.create_thread(thread(public.id)).await.unwrap(),
                store.create_thread(thread(secret.id)).await.unwrap(),
            )
        }
    };
    let (ws, agent, public, public_sibling, secret) = seed("a").await;
    let (other_ws, _, other, _, _) = seed("b").await;

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(bus));
    let auth =
        AuthContext::from_session(agent.id, ws, caps.iter().map(|c| c.to_string()).collect());
    WaitFixture {
        server,
        store,
        auth,
        agent,
        ws,
        public,
        public_sibling,
        secret,
        other_ws,
        other,
    }
}

fn tool_content(v: Value) -> Value {
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn wait_for_claim_failed_returns_the_stop_and_sees_no_other_tenant() {
    use chrono::Utc;
    use maidan_auth::capability::WORKSPACE_READ;
    use std::time::Duration;

    let f = wait_fixture(&[WORKSPACE_READ]).await;
    let failed = |ws: WorkspaceId, thread: &Thread, member_id: MemberId| Event::ClaimFailed {
        occurred_at: Utc::now(),
        workspace_id: ws,
        channel_id: thread.channel_id,
        thread_id: thread.id,
        member_id,
        reason: "tokens".into(),
        thread: thread.clone(),
    };
    let call = |args: Value| {
        let (server, auth) = (&f.server, &f.auth);
        async move {
            tool_content(
                server
                    .call_tool(auth, "wait_for_claim_failed", &args)
                    .await
                    .unwrap(),
            )
        }
    };

    // A stop in the public thread wakes the workspace-wide waiter.
    let (got, _) = tokio::join!(call(json!({ "timeout_ms": 5000 })), async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        f.server
            .publish_event(failed(f.ws, &f.public, f.agent.id))
            .await;
    });
    assert_eq!(got["kind"], json!("claim_failed"));
    assert_eq!(got["thread_id"], json!(f.public.id.0));
    assert_eq!(got["member_id"], json!(f.agent.id.0));
    assert_eq!(got["reason"], json!("tokens"));

    // Nothing happens: the wait returns null once its window lapses.
    let started = tokio::time::Instant::now();
    assert!(call(json!({ "timeout_ms": 200 })).await.is_null());
    assert!(started.elapsed() >= Duration::from_millis(200));

    // A stop in a private channel the waiter can't read, or in another
    // workspace, is never delivered.
    let (got, _) = tokio::join!(call(json!({ "timeout_ms": 400 })), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        f.server
            .publish_event(failed(f.ws, &f.secret, f.agent.id))
            .await;
        f.server
            .publish_event(failed(f.other_ws, &f.other, f.agent.id))
            .await;
    });
    assert!(got.is_null(), "a private or foreign stop leaked: {got}");

    // A stop before the call is caught with since_log_id; the lookback is
    // held to the same rules.
    let id = f
        .server
        .publish_event(failed(f.ws, &f.public, f.agent.id))
        .await
        .unwrap();
    let got = call(json!({ "timeout_ms": 300, "since_log_id": id - 1 })).await;
    assert_eq!(got["kind"], json!("claim_failed"));
    for (ws, thread) in [(f.ws, &f.secret), (f.other_ws, &f.other)] {
        let id = f
            .server
            .publish_event(failed(ws, thread, f.agent.id))
            .await
            .unwrap();
        assert!(call(json!({ "timeout_ms": 300, "since_log_id": id - 1 }))
            .await
            .is_null());
    }

    // Naming a channel the caller can't read is refused before it parks.
    for channel_id in [f.secret.channel_id, f.other.channel_id] {
        let refused = f
            .server
            .call_tool(
                &f.auth,
                "wait_for_claim_failed",
                &json!({ "channel_id": channel_id.0, "timeout_ms": 300 }),
            )
            .await;
        assert!(refused.is_err(), "channel {channel_id:?} was not refused");
    }
}

/// A one-slot bus drops all but the newest of a burst, so the waiter sees a
/// lag marker. The log replay after it still answers the first match, not
/// a later one that the caller's next `after_log_id` would skip past.
/// (Found by the red-team review of #1488.)
#[tokio::test]
async fn a_lagged_wait_answers_the_first_match_from_the_log() {
    use chrono::Utc;
    use maidan_auth::capability::WORKSPACE_READ;
    use std::time::Duration;

    let f = wait_fixture_on(&[WORKSPACE_READ], maidan_bus::InMemoryBus::with_capacity(1)).await;
    let bus = f.server.event_bus.clone().unwrap();
    let failed = |thread: &Thread| Event::ClaimFailed {
        occurred_at: Utc::now(),
        workspace_id: f.ws,
        channel_id: thread.channel_id,
        thread_id: thread.id,
        member_id: f.agent.id,
        reason: "tokens".into(),
        thread: thread.clone(),
    };
    let resolved = |thread: &Thread| Event::BlockedResolved {
        occurred_at: Utc::now(),
        workspace_id: f.ws,
        channel_id: thread.channel_id,
        thread_id: thread.id,
        reason: BlockedReason::Human,
        resolved_by: f.agent.id,
    };
    // A burst after the wait parks: the first match, two other kinds, then
    // a second match. With one slot, the first match is dropped.
    let burst = || async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let first = f.store.append_event(&failed(&f.public)).await.unwrap();
        let x = f.store.append_event(&resolved(&f.public)).await.unwrap();
        let y = f.store.append_event(&resolved(&f.public)).await.unwrap();
        let second = f
            .store
            .append_event(&failed(&f.public_sibling))
            .await
            .unwrap();
        for stored in [&first, &x, &y, &second] {
            bus.publish(BusEnvelope::from_stored(stored).unwrap())
                .await
                .unwrap();
        }
        first.id
    };

    // With after_log_id, and without one.
    let start = f.store.append_event(&resolved(&f.public)).await.unwrap().id;
    for args in [
        json!({ "kinds": ["claim_failed"], "after_log_id": start, "timeout_ms": 3000 }),
        json!({ "kinds": ["claim_failed"], "timeout_ms": 3000 }),
    ] {
        let (got, first) = tokio::join!(
            f.server.call_tool(&f.auth, "wait_for_event", &args),
            burst()
        );
        let got = tool_content(got.unwrap());
        assert_eq!(got["log_id"].as_i64(), Some(first), "{args}: {got}");
    }

    // A single-kind wait shares the path.
    let args = json!({ "timeout_ms": 3000 });
    let (got, _) = tokio::join!(
        f.server.call_tool(&f.auth, "wait_for_claim_failed", &args),
        burst()
    );
    let got = tool_content(got.unwrap());
    assert_eq!(got["thread_id"], json!(f.public.id.0), "{got}");
}

#[tokio::test]
async fn wait_for_event_wakes_on_a_named_kind_it_may_read_and_on_nothing_else() {
    use chrono::Utc;
    use maidan_auth::capability::WORKSPACE_READ;
    use std::time::Duration;

    let f = wait_fixture(&[WORKSPACE_READ]).await;
    let failed = |ws: WorkspaceId, thread: &Thread| Event::ClaimFailed {
        occurred_at: Utc::now(),
        workspace_id: ws,
        channel_id: thread.channel_id,
        thread_id: thread.id,
        member_id: f.agent.id,
        reason: "tokens".into(),
        thread: thread.clone(),
    };
    let resolved = |thread: &Thread| Event::BlockedResolved {
        occurred_at: Utc::now(),
        workspace_id: f.ws,
        channel_id: thread.channel_id,
        thread_id: thread.id,
        reason: BlockedReason::Human,
        resolved_by: f.agent.id,
    };
    let secret_channel = f.store.get_channel(f.secret.channel_id).await.unwrap();
    let call = |args: Value| {
        let (server, auth) = (&f.server, &f.auth);
        async move {
            tool_content(
                server
                    .call_tool(auth, "wait_for_event", &args)
                    .await
                    .unwrap(),
            )
        }
    };
    let kinds = json!(["claim_failed", "channel_created"]);

    // A named kind published during the wait wakes it, with its log id;
    // another kind first does not.
    let (got, _) = tokio::join!(call(json!({ "kinds": kinds, "timeout_ms": 5000 })), async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        f.server.publish_event(resolved(&f.public)).await;
        f.server.publish_event(failed(f.ws, &f.public)).await;
    });
    assert_eq!(got["event"]["kind"], json!("claim_failed"), "{got}");
    assert_eq!(got["event"]["thread_id"], json!(f.public.id.0));
    assert!(got["log_id"].as_i64().is_some(), "{got}");

    // Nothing happens: null once the fixed window lapses.
    let started = tokio::time::Instant::now();
    assert!(call(json!({ "kinds": kinds, "timeout_ms": 200 }))
        .await
        .is_null());
    assert!(started.elapsed() >= Duration::from_millis(200));

    // A private channel's thread event, a private channel's own event, and
    // another workspace's event never wake it.
    let (got, _) = tokio::join!(call(json!({ "kinds": kinds, "timeout_ms": 400 })), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        f.server.publish_event(failed(f.ws, &f.secret)).await;
        f.server
            .publish_event(Event::ChannelCreated {
                occurred_at: Utc::now(),
                workspace_id: f.ws,
                channel: secret_channel.clone(),
            })
            .await;
        f.server.publish_event(failed(f.other_ws, &f.other)).await;
    });
    assert!(got.is_null(), "a private or foreign event leaked: {got}");

    // A thread scope narrows it to that thread.
    let (got, _) = tokio::join!(
        call(json!({ "kinds": kinds, "thread_id": f.public_sibling.id.0, "timeout_ms": 5000 })),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            f.server.publish_event(failed(f.ws, &f.public)).await;
            f.server
                .publish_event(failed(f.ws, &f.public_sibling))
                .await;
        }
    );
    assert_eq!(
        got["event"]["thread_id"],
        json!(f.public_sibling.id.0),
        "{got}"
    );

    // after_log_id replays what happened before the call, under the same
    // rules; timeout_ms 0 only replays.
    let id = f
        .server
        .publish_event(failed(f.ws, &f.public))
        .await
        .unwrap();
    let got = call(json!({ "kinds": kinds, "timeout_ms": 0, "after_log_id": id - 1 })).await;
    assert_eq!(got["log_id"], json!(id), "{got}");
    for (ws, thread) in [(f.ws, &f.secret), (f.other_ws, &f.other)] {
        let id = f.server.publish_event(failed(ws, thread)).await.unwrap();
        assert!(
            call(json!({ "kinds": kinds, "timeout_ms": 0, "after_log_id": id - 1 }))
                .await
                .is_null()
        );
    }

    // Refused before it parks: an unknown kind, a thread it can't read,
    // another workspace.
    for args in [
        json!({ "kinds": ["claim_failed", "no_such_kind"] }),
        json!({ "kinds": kinds, "thread_id": f.secret.id.0 }),
        json!({ "kinds": kinds, "thread_id": f.other.id.0 }),
        json!({ "kinds": kinds, "workspace_id": f.other_ws.0 }),
    ] {
        let refused = f.server.call_tool(&f.auth, "wait_for_event", &args).await;
        assert!(refused.is_err(), "{args} was not refused");
    }
}

#[tokio::test]
async fn wait_for_blocked_resolved_returns_the_clear_and_sees_no_other_tenant() {
    use chrono::Utc;
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use std::time::Duration;

    let f = wait_fixture(&[WORKSPACE_READ, THREAD_TRANSITION]).await;
    let resolved = |ws: WorkspaceId, thread: &Thread| Event::BlockedResolved {
        occurred_at: Utc::now(),
        workspace_id: ws,
        channel_id: thread.channel_id,
        thread_id: thread.id,
        reason: BlockedReason::Human,
        resolved_by: f.agent.id,
    };
    let call = |args: Value| {
        let (server, auth) = (&f.server, &f.auth);
        async move {
            server
                .call_tool(auth, "wait_for_blocked_resolved", &args)
                .await
        }
    };

    // Clearing a real block wakes a waiter scoped to that thread.
    f.store
        .set_thread_block(f.public.id, BlockedReason::Human, f.agent.id, None)
        .await
        .unwrap();
    let (got, _) = tokio::join!(
        call(json!({ "thread_id": f.public.id.0, "timeout_ms": 5000 })),
        async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            f.server
                .call_tool(
                    &f.auth,
                    "clear_thread_block",
                    &json!({ "thread_id": f.public.id.0 }),
                )
                .await
                .unwrap();
        }
    );
    let got = tool_content(got.unwrap());
    assert_eq!(got["kind"], json!("blocked_resolved"));
    assert_eq!(got["thread_id"], json!(f.public.id.0));
    assert_eq!(got["reason"], json!("human"));
    assert_eq!(got["resolved_by"], json!(f.agent.id.0));

    // Nothing happens: null once the window lapses.
    let started = tokio::time::Instant::now();
    assert!(tool_content(call(json!({ "timeout_ms": 200 })).await.unwrap()).is_null());
    assert!(started.elapsed() >= Duration::from_millis(200));

    // A clear on a sibling thread does not satisfy a thread-scoped wait; a
    // private or foreign clear does not satisfy a workspace-wide one.
    let (scoped, wide, _) = tokio::join!(
        call(json!({ "thread_id": f.public.id.0, "timeout_ms": 400 })),
        call(json!({ "timeout_ms": 400 })),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            f.server
                .publish_event(resolved(f.ws, &f.public_sibling))
                .await;
            f.server.publish_event(resolved(f.ws, &f.secret)).await;
            f.server.publish_event(resolved(f.other_ws, &f.other)).await;
        }
    );
    assert!(tool_content(scoped.unwrap()).is_null());
    let wide = tool_content(wide.unwrap());
    assert_eq!(
        wide["thread_id"],
        json!(f.public_sibling.id.0),
        "only the readable clear in this workspace is delivered"
    );

    // A clear before the call is caught with since_log_id; the lookback
    // skips private and foreign clears.
    let id = f
        .server
        .publish_event(resolved(f.ws, &f.public))
        .await
        .unwrap();
    let got = tool_content(
        call(json!({ "thread_id": f.public.id.0, "timeout_ms": 300, "since_log_id": id - 1 }))
            .await
            .unwrap(),
    );
    assert_eq!(got["kind"], json!("blocked_resolved"));
    let secret_id = f
        .server
        .publish_event(resolved(f.ws, &f.secret))
        .await
        .unwrap();
    f.server
        .publish_event(resolved(f.other_ws, &f.other))
        .await
        .unwrap();
    assert!(tool_content(
        call(json!({ "timeout_ms": 300, "since_log_id": secret_id - 1 }))
            .await
            .unwrap()
    )
    .is_null());

    // Naming a thread or channel the caller can't read is refused before
    // it parks, rather than waiting out a window that can never fire.
    for args in [
        json!({ "thread_id": f.secret.id.0, "timeout_ms": 300 }),
        json!({ "thread_id": f.other.id.0, "timeout_ms": 300 }),
        json!({ "channel_id": f.other.channel_id.0, "timeout_ms": 300 }),
    ] {
        assert!(call(args.clone()).await.is_err(), "{args} was not refused");
    }
}
