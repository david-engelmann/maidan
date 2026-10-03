//! A workspace sets its own retention, within what the instance keeps.
//!
//! Over REST and MCP: `token:admin` sets it, a value longer than the instance
//! keeps is refused, and one workspace's administrator can neither read nor set
//! another's. Through the sweeper, on both backends: a workspace with a
//! one-day policy loses its old messages, events and finished deliveries, a
//! workspace without one keeps its rows, and a held workspace keeps its rows
//! whatever its policy says. Its own test binary: it sets the instance's
//! retention in the environment.

use std::{
    future::Future,
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{retention, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, ChannelId, Event, Member, MemberKind, MessageId, NewApiToken, NewAuditEvent,
    NewChannel, NewDlqEntry, NewMember, NewMessage, NewThread, NewWorkspace, RetentionDays,
    ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

/// The instance keeps events 30 days and messages 14 days; a workspace may
/// keep either for less time, not more.
const INSTANCE_EVENTS_DAYS: &str = "30";
const INSTANCE_MESSAGES_DAYS: &str = "14";

async fn sqlite_store() -> (Arc<dyn Store>, sqlx::SqlitePool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    (Arc::new(SqliteStore::for_tests(pool.clone())), pool)
}

async fn spawn() -> (String, reqwest::Client, Arc<dyn Store>) {
    unsafe {
        std::env::set_var("MAIDAN_RETENTION_EVENTS_DAYS", INSTANCE_EVENTS_DAYS);
        std::env::set_var("MAIDAN_RETENTION_MESSAGES_DAYS", INSTANCE_MESSAGES_DAYS);
        std::env::remove_var("MAIDAN_RETENTION_DELIVERIES_DAYS");
    }
    let (store, pool) = sqlite_store().await;
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    std::mem::forget(dir);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), reqwest::Client::new(), store)
}

struct Tenant {
    ws: WorkspaceId,
    admin: String,
    reader: String,
}

async fn tenant(store: &Arc<dyn Store>, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let mut tokens = Vec::new();
    for (handle, caps) in [
        (
            "admin",
            vec![capability::TOKEN_ADMIN, capability::WORKSPACE_READ],
        ),
        ("reader", vec![capability::WORKSPACE_READ]),
    ] {
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let secret = TokenSecret::generate();
        store
            .create_api_token(NewApiToken {
                workspace_id: ws.id,
                member_id: member.id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: caps.into_iter().map(String::from).collect(),
                expires_at: None,
            })
            .await
            .unwrap();
        tokens.push(secret.as_str().to_string());
    }
    Tenant {
        ws: ws.id,
        admin: tokens[0].clone(),
        reader: tokens[1].clone(),
    }
}

async fn mcp(client: &reqwest::Client, base: &str, token: &str, tool: &str, args: Value) -> Value {
    client
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": args }
        }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()
}

fn tool_result(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no tool result: {response}"));
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn a_workspace_sets_its_own_retention_within_the_instances_over_rest_and_mcp() {
    let (base, client, store) = spawn().await;
    let a = tenant(&store, "a").await;
    let b = tenant(&store, "b").await;
    let url = |ws: WorkspaceId| format!("{base}/workspaces/{}/retention", ws.0);

    let read: Value = client
        .get(url(a.ws))
        .bearer_auth(&a.reader)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        read["workspace"],
        json!({"messages_days": null, "events_days": null, "deliveries_days": null})
    );
    assert_eq!(read["instance"]["events_days"], 30);
    assert_eq!(read["instance"]["messages_days"], 14);
    assert_eq!(read["effective"]["events_days"], 30);
    assert_eq!(read["effective"]["messages_days"], 14);

    let as_reader = client
        .put(url(a.ws))
        .bearer_auth(&a.reader)
        .json(&json!({ "messages_days": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        as_reader.status(),
        StatusCode::FORBIDDEN,
        "setting it is token:admin"
    );

    let too_long = client
        .put(url(a.ws))
        .bearer_auth(&a.admin)
        .json(&json!({ "events_days": 31 }))
        .send()
        .await
        .unwrap();
    assert_eq!(too_long.status(), StatusCode::BAD_REQUEST);
    assert!(too_long.text().await.unwrap().contains("events_days"));

    let messages_too_long = client
        .put(url(a.ws))
        .bearer_auth(&a.admin)
        .json(&json!({ "messages_days": 15 }))
        .send()
        .await
        .unwrap();
    assert_eq!(messages_too_long.status(), StatusCode::BAD_REQUEST);
    assert!(messages_too_long
        .text()
        .await
        .unwrap()
        .contains("messages_days"));

    let unknown = client
        .put(url(a.ws))
        .bearer_auth(&a.admin)
        .json(&json!({ "audit_days": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        unknown.status().as_u16() / 100,
        4,
        "a workspace has no audit retention to set"
    );

    let set = client
        .put(url(a.ws))
        .bearer_auth(&a.admin)
        .json(&json!({ "messages_days": 7, "events_days": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);
    let set: Value = set.json().await.unwrap();
    assert_eq!(set["workspace"]["messages_days"], 7);
    assert_eq!(set["effective"]["events_days"], 1);
    assert_eq!(set["effective"]["deliveries_days"], Value::Null);

    let audit = store.list_audit_for_workspace(a.ws, 50).await.unwrap();
    let row = audit
        .iter()
        .find(|row| row.action == "retention_policy.set")
        .expect("the change is audited in the workspace");
    assert_eq!(row.metadata["messages_days"], 7);

    // Another workspace's administrator can neither set nor read A's, and A's
    // policy is not B's.
    for response in [
        client
            .put(url(a.ws))
            .bearer_auth(&b.admin)
            .json(&json!({ "messages_days": 1 }))
            .send()
            .await
            .unwrap(),
        client
            .get(url(a.ws))
            .bearer_auth(&b.admin)
            .send()
            .await
            .unwrap(),
    ] {
        assert!(
            matches!(
                response.status(),
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
            ),
            "B's token on A's retention: {}",
            response.status()
        );
    }
    assert_eq!(
        store
            .get_retention_policy(a.ws)
            .await
            .unwrap()
            .messages_days,
        Some(7),
        "B's attempt changed nothing"
    );
    assert!(store.get_retention_policy(b.ws).await.unwrap().is_unset());

    // The MCP twins act on the caller's own workspace, on the same terms.
    let b_read =
        tool_result(&mcp(&client, &base, &b.reader, "get_retention_policy", json!({})).await);
    assert_eq!(b_read["workspace_id"], json!(b.ws.0));
    assert_eq!(b_read["workspace"]["messages_days"], Value::Null);
    let denied = mcp(
        &client,
        &base,
        &b.reader,
        "set_retention_policy",
        json!({ "messages_days": 1 }),
    )
    .await;
    assert!(denied["error"].is_object(), "{denied}");
    let refused = mcp(
        &client,
        &base,
        &b.admin,
        "set_retention_policy",
        json!({ "events_days": 31 }),
    )
    .await;
    assert!(
        refused["error"].is_object() || refused["result"]["isError"] == true,
        "longer than the instance keeps: {refused}"
    );
    let b_set = tool_result(
        &mcp(
            &client,
            &base,
            &b.admin,
            "set_retention_policy",
            json!({ "deliveries_days": 3 }),
        )
        .await,
    );
    assert_eq!(b_set["workspace"]["deliveries_days"], 3);
    assert_eq!(
        store
            .get_retention_policy(a.ws)
            .await
            .unwrap()
            .deliveries_days,
        None,
        "B's MCP call set B's policy, not A's"
    );
}

struct Room {
    workspace: WorkspaceId,
    channel: ChannelId,
    thread: ThreadId,
    member: Member,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
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
            title: Some("work".into()),
        })
        .await
        .unwrap();
    Room {
        workspace: ws.id,
        channel: channel.id,
        thread: thread.id,
        member,
    }
}

/// Old rows in a room: a message, an event and a dead-lettered run.
async fn old_rows(store: &dyn Store, room: &Room) -> (MessageId, i64) {
    let message = store
        .post_message(NewMessage {
            thread_id: room.thread,
            author_id: room.member.id,
            body: "old words".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap()
        .id;
    let event = store
        .append_event(&Event::MemberJoined {
            occurred_at: Utc::now() - chrono::Duration::days(10),
            workspace_id: room.workspace,
            member: room.member.clone(),
        })
        .await
        .unwrap()
        .id;
    store
        .record_dlq_entry(&NewDlqEntry {
            workspace_id: room.workspace,
            channel_id: room.channel,
            thread_id: room.thread,
            member_id: room.member.id,
            reason: "budget".into(),
            used_tokens: 1,
            used_usd_micros: 1,
            used_turns: 1,
        })
        .await
        .unwrap();
    (message, event)
}

fn one_day() -> RetentionDays {
    RetentionDays {
        messages_days: Some(1),
        events_days: Some(1),
        deliveries_days: Some(1),
    }
}

async fn set_policy(store: &dyn Store, ws: WorkspaceId) {
    store
        .set_retention_policy_audited(
            ws,
            one_day(),
            RetentionDays::default(),
            Box::new(move |_| NewAuditEvent {
                scope: AuditScope::Workspace(ws),
                actor_id: None,
                action: "retention_policy.set".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(ws.0),
                metadata: json!({}),
            }),
        )
        .await
        .unwrap();
}

/// `backdate` moves every stored message and dead letter ten days back, in
/// the database's own clock.
async fn run_sweep_suite<F, Fut>(store: Arc<dyn Store>, backdate: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    let a = room(store.as_ref(), "one-day").await;
    let b = room(store.as_ref(), "no-policy").await;
    let h = room(store.as_ref(), "held").await;
    let a_old = old_rows(store.as_ref(), &a).await;
    let b_old = old_rows(store.as_ref(), &b).await;
    let h_old = old_rows(store.as_ref(), &h).await;
    backdate().await;
    set_policy(store.as_ref(), a.workspace).await;
    set_policy(store.as_ref(), h.workspace).await;
    store
        .place_legal_hold(h.workspace, "matter", None)
        .await
        .unwrap();

    // The instance keeps everything; only A's policy prunes.
    let cfg = retention::RetentionConfig {
        messages_days: None,
        events_days: None,
        audit_days: None,
        deliveries_days: None,
        notifications_days: None,
        sweep: Duration::from_secs(86_400),
        batch: 100,
    };
    retention::sweep_once(&store, &cfg).await;

    assert!(
        store.get_message(a_old.0).await.is_err(),
        "A's old message is erased"
    );
    assert!(
        store.get_stored_event(a_old.1).await.is_err(),
        "A's old event is pruned"
    );
    assert!(store
        .list_channel_dlq(a.channel, 10)
        .await
        .unwrap()
        .is_empty());

    for (room, (message, event), why) in [
        (&b, b_old, "B set no policy"),
        (&h, h_old, "H is held, whatever its policy says"),
    ] {
        assert_eq!(
            store.get_message(message).await.unwrap().body,
            "old words",
            "{why}"
        );
        assert!(store.get_stored_event(event).await.is_ok(), "{why}");
        assert_eq!(
            store
                .list_channel_dlq(room.channel, 10)
                .await
                .unwrap()
                .len(),
            1,
            "{why}"
        );
    }
}

#[tokio::test]
async fn the_sweeper_prunes_only_the_workspace_whose_policy_says_so_sqlite() {
    let (store, pool) = sqlite_store().await;
    run_sweep_suite(store, || async {
        for sql in [
            "UPDATE maidan_messages SET posted_at = strftime('%Y-%m-%dT%H:%M:%fZ', posted_at, '-10 days')",
            "UPDATE maidan_agent_work_dlq SET failed_at = strftime('%Y-%m-%dT%H:%M:%fZ', failed_at, '-10 days')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
    })
    .await;
}

#[tokio::test]
async fn the_sweeper_prunes_only_the_workspace_whose_policy_says_so_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    run_sweep_suite(store, || async {
        for sql in [
            "UPDATE maidan_messages SET posted_at = posted_at - INTERVAL '10 days'",
            "UPDATE maidan_agent_work_dlq SET failed_at = failed_at - INTERVAL '10 days'",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
    })
    .await;
}

/// The instance message ceiling prunes a workspace that set no policy. A
/// message posted after the cutoff stays, and a held workspace keeps its old
/// message.
async fn run_instance_message_ceiling<F, Fut>(store: Arc<dyn Store>, backdate: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    let open = room(store.as_ref(), "ceiling-open").await;
    let held = room(store.as_ref(), "ceiling-held").await;
    let (open_old, _) = old_rows(store.as_ref(), &open).await;
    let (held_old, _) = old_rows(store.as_ref(), &held).await;
    backdate().await;
    let fresh = store
        .post_message(NewMessage {
            thread_id: open.thread,
            author_id: open.member.id,
            body: "fresh words".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap()
        .id;
    store
        .place_legal_hold(held.workspace, "matter", None)
        .await
        .unwrap();

    let cfg = retention::RetentionConfig {
        messages_days: Some(1),
        events_days: None,
        audit_days: None,
        deliveries_days: None,
        notifications_days: None,
        sweep: Duration::from_secs(86_400),
        batch: 100,
    };
    retention::sweep_once(&store, &cfg).await;

    assert!(
        store.get_message(open_old).await.is_err(),
        "the instance ceiling erased the old message"
    );
    assert_eq!(
        store.get_message(fresh).await.unwrap().body,
        "fresh words",
        "a message inside the ceiling stays"
    );
    assert_eq!(
        store.get_message(held_old).await.unwrap().body,
        "old words",
        "a held workspace keeps the old message"
    );
}

#[tokio::test]
async fn the_instance_message_ceiling_prunes_without_a_workspace_policy_sqlite() {
    let (store, pool) = sqlite_store().await;
    run_instance_message_ceiling(store, || async {
        sqlx::query(
            "UPDATE maidan_messages SET posted_at = strftime('%Y-%m-%dT%H:%M:%fZ', posted_at, '-10 days')",
        )
        .execute(&pool)
        .await
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn the_instance_message_ceiling_prunes_without_a_workspace_policy_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    run_instance_message_ceiling(store, || async {
        sqlx::query("UPDATE maidan_messages SET posted_at = posted_at - INTERVAL '10 days'")
            .execute(&pool)
            .await
            .unwrap();
    })
    .await;
}
