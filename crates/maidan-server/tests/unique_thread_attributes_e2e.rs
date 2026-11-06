//! Unique thread attributes over the wire: the workspace policy route (read
//! by members, set by an admin, audited, closed to another tenant), and the
//! 409 a second open thread on a held ticket gets on REST and MCP, at create
//! and at a later attribute set, naming the holder only to a caller who may
//! read it.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace,
    TaskAttributes, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

async fn spawn() -> (String, Arc<dyn Store>) {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), store)
}

async fn member_with(
    store: &dyn Store,
    ws: WorkspaceId,
    handle: &str,
    capabilities: &[&str],
) -> (MemberId, String) {
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: capabilities.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    (member.id, secret.as_str().to_string())
}

const CAPS: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::THREAD_TRANSITION,
];

const ADMIN: &[&str] = &[capability::TOKEN_ADMIN, capability::WORKSPACE_READ];

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    token: String,
    admin: String,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let (_member, token) = member_with(store, ws.id, "agent", CAPS).await;
    let (_admin, admin) = member_with(store, ws.id, "admin", ADMIN).await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        ws: ws.id,
        channel: channel.id,
        token,
        admin,
    }
}

async fn policy(
    base: &str,
    token: &str,
    ws: WorkspaceId,
    put: Option<Value>,
) -> (StatusCode, Value) {
    let url = format!("{base}/workspaces/{}/unique-thread-attributes", ws.0);
    let client = reqwest::Client::new();
    let request = match put {
        Some(body) => client.put(url).json(&body),
        None => client.get(url),
    };
    let response = request.bearer_auth(token).send().await.unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn keep_tickets_unique(base: &str, room: &Room) {
    let (status, body) = policy(
        base,
        &room.admin,
        room.ws,
        Some(json!({ "keys": ["linear_issue"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn create_thread(
    base: &str,
    token: &str,
    channel: ChannelId,
    body: Value,
) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", channel.0))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn set_attributes(base: &str, token: &str, thread: &str, body: Value) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .put(format!("{base}/threads/{thread}/attributes"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn tool(base: &str, token: &str, name: &str, arguments: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .header("accept", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// The JSON a successful tool call returned.
fn tool_result(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("not a tool result: {response}"));
    serde_json::from_str(text).unwrap()
}

/// A REST 409 `conflict` for a held ticket.
fn assert_held(status: StatusCode, body: &Value) {
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["type"], "https://maidan.world/problems/conflict",
        "{body}"
    );
    assert!(
        body["detail"].as_str().unwrap_or("").contains("ENG-6598"),
        "{body}"
    );
}

/// The MCP twin of [`assert_held`].
fn assert_tool_held(response: &Value) {
    assert_eq!(response["error"]["code"], -32011, "{response}");
    assert_eq!(response["error"]["data"]["status"], 409, "{response}");
    assert_eq!(
        response["error"]["data"]["type"], "https://maidan.world/problems/conflict",
        "{response}"
    );
}

#[tokio::test]
async fn a_workspace_admin_sets_the_unique_attributes_and_the_change_is_audited() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "policy").await;

    let (status, unset) = policy(&base, &room.token, room.ws, None).await;
    assert_eq!(status, StatusCode::OK, "a member may read it");
    assert_eq!(unset, json!({ "keys": [] }));
    let tickets = json!({ "keys": ["linear_issue", "linear_issue"] });
    let (status, _) = policy(&base, &room.token, room.ws, Some(tickets.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "it is an admin's call");
    let (status, set) = policy(&base, &room.admin, room.ws, Some(tickets)).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!(
        set,
        json!({ "keys": ["linear_issue"] }),
        "stored without repeats"
    );
    let (_, read) = policy(&base, &room.token, room.ws, None).await;
    assert_eq!(read, json!({ "keys": ["linear_issue"] }));
    let audit = store
        .list_audit_for_workspace(room.ws, 50)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.action == "workspace.unique_thread_attributes.set")
        .expect("the change is audited");
    assert_eq!(audit.metadata, json!({ "keys": ["linear_issue"] }));
    for bad in [
        json!({ "keys": ["kind"] }),
        json!({ "key": ["linear_issue"] }),
    ] {
        let (status, body) = policy(&base, &room.admin, room.ws, Some(bad)).await;
        assert!(status.is_client_error(), "refused: {status} {body}");
    }
    let (_, read) = policy(&base, &room.token, room.ws, None).await;
    assert_eq!(
        read,
        json!({ "keys": ["linear_issue"] }),
        "a refused PUT changes nothing"
    );
}

#[tokio::test]
async fn another_workspace_cannot_read_or_set_the_unique_attributes() {
    let (base, store) = spawn().await;
    let victim = room(store.as_ref(), "victim").await;
    let attacker = room(store.as_ref(), "attacker").await;
    keep_tickets_unique(&base, &victim).await;
    let (_, attacker_own) = policy(&base, &attacker.admin, attacker.ws, None).await;
    assert_eq!(
        attacker_own,
        json!({ "keys": [] }),
        "the setting stays in its workspace"
    );
    for put in [None, Some(json!({ "keys": [] }))] {
        let (status, body) = policy(&base, &attacker.admin, victim.ws, put).await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "another tenant must be refused, got {status}: {body}"
        );
    }
    let (_, read) = policy(&base, &victim.admin, victim.ws, None).await;
    assert_eq!(
        read,
        json!({ "keys": ["linear_issue"] }),
        "the victim's setting holds"
    );

    // The attacker's own threads on the victim's ticket are not refused.
    let ticket = json!({ "title": "t", "attributes": { "linear_issue": "ENG-6598" } });
    let (status, _) = create_thread(&base, &victim.token, victim.channel, ticket.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    keep_tickets_unique(&base, &attacker).await;
    let (status, body) = create_thread(&base, &attacker.token, attacker.channel, ticket).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test]
async fn a_held_ticket_is_a_409_naming_the_holder_on_rest_and_mcp() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "held").await;
    keep_tickets_unique(&base, &room).await;
    let ticket = json!({ "linear_issue": "ENG-6598" });

    let (status, pi) = create_thread(
        &base,
        &room.token,
        room.channel,
        json!({ "title": "pi draft", "attributes": ticket }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pi}");
    let holder = pi["id"].clone();

    // A second create on the ticket, over REST and over MCP.
    let (status, refused) = create_thread(
        &base,
        &room.token,
        room.channel,
        json!({ "title": "laptop session", "attributes": ticket }),
    )
    .await;
    assert_held(status, &refused);
    assert_eq!(refused["holder_thread_id"], holder, "{refused}");
    let refused = tool(
        &base,
        &room.token,
        "create_thread",
        json!({ "channel_id": room.channel.0, "title": "mcp", "attributes": ticket }),
    )
    .await;
    assert_tool_held(&refused);
    assert_eq!(
        refused["error"]["data"]["holder_thread_id"], holder,
        "{refused}"
    );

    // Setting the ticket on another open thread later, over REST and MCP.
    let (status, other) = create_thread(
        &base,
        &room.token,
        room.channel,
        json!({ "title": "other" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let other = other["id"].as_str().unwrap().to_string();
    let (status, refused) = set_attributes(&base, &room.token, &other, ticket.clone()).await;
    assert_held(status, &refused);
    assert_eq!(refused["holder_thread_id"], holder, "{refused}");
    let refused = tool(
        &base,
        &room.token,
        "set_thread_attributes",
        json!({ "thread_id": other, "kind": "change", "linear_issue": "ENG-6598" }),
    )
    .await;
    assert_tool_held(&refused);
    assert_eq!(
        refused["error"]["data"]["holder_thread_id"], holder,
        "{refused}"
    );

    // Another ticket is free on both.
    let (status, body) = set_attributes(
        &base,
        &room.token,
        &other,
        json!({ "linear_issue": "ENG-1" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let made = tool_result(
        &tool(
            &base,
            &room.token,
            "create_thread",
            json!({ "channel_id": room.channel.0, "title": "free", "attributes": { "linear_issue": "ENG-2" } }),
        )
        .await,
    );
    assert_eq!(made["attributes"]["linear_issue"], "ENG-2");
}

#[tokio::test]
async fn a_holder_the_caller_cannot_read_is_not_named() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "private-holder").await;
    keep_tickets_unique(&base, &room).await;
    let private = store
        .create_channel(NewChannel {
            workspace_id: room.ws,
            name: "private".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    store
        .create_thread_with_footprint(
            NewThread {
                channel_id: private.id,
                parent_thread_id: None,
                title: Some("held in private".into()),
                description: None,
                created_by: None,
            },
            None,
            Some(TaskAttributes {
                linear_issue: Some("ENG-6598".into()),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .unwrap();

    let ticket = json!({ "linear_issue": "ENG-6598" });
    let (status, refused) = create_thread(
        &base,
        &room.token,
        room.channel,
        json!({ "title": "public", "attributes": ticket }),
    )
    .await;
    assert_held(status, &refused);
    assert!(refused.get("holder_thread_id").is_none(), "{refused}");
    let refused = tool(
        &base,
        &room.token,
        "create_thread",
        json!({ "channel_id": room.channel.0, "title": "mcp", "attributes": ticket }),
    )
    .await;
    assert_tool_held(&refused);
    assert!(
        refused["error"]["data"].get("holder_thread_id").is_none(),
        "{refused}"
    );

    let (_, other) = create_thread(
        &base,
        &room.token,
        room.channel,
        json!({ "title": "other" }),
    )
    .await;
    let other = other["id"].as_str().unwrap().to_string();
    let (status, refused) = set_attributes(&base, &room.token, &other, ticket).await;
    assert_held(status, &refused);
    assert!(refused.get("holder_thread_id").is_none(), "{refused}");
}
