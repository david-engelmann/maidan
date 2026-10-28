//! Task fingerprints over the wire (Context Economics C8): read on the
//! thread, linked to an identical open task, invisible to another tenant.

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
    ChannelId, ChannelMemberRole, MemberId, MemberKind, NewApiToken, NewChannel, NewMember,
    NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (String, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
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

struct Room {
    channel: ChannelId,
    token: String,
}

async fn room(base_store: &dyn Store, name: &str) -> Room {
    let ws = base_store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let (_member, token) = member_with(base_store, ws.id, "agent", CAPS).await;
    let channel = base_store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        channel: channel.id,
        token,
    }
}

async fn create_thread(base: &str, room: &Room, body: Value) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", room.channel.0))
        .bearer_auth(&room.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn get_fingerprint(base: &str, token: &str, thread_id: &str) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .get(format!("{base}/threads/{thread_id}/fingerprint"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

/// A second identical task reads back linked to the first; another tenant
/// cannot read either fingerprint.
#[tokio::test]
async fn an_identical_task_reads_back_linked_and_is_invisible_to_another_tenant() {
    let (base, store) = spawn().await;
    let home = room(store.as_ref(), "home").await;
    let away = room(store.as_ref(), "away").await;
    let brief =
        json!({ "title": "Fix the sweep", "attributes": { "acceptance": "the sweep closes it" } });
    let (status, first) = create_thread(&base, &home, brief.clone()).await;
    assert!(status.is_success(), "{status} {first}");
    let (status, second) = create_thread(&base, &home, brief).await;
    assert!(status.is_success(), "{status} {second}");
    let first_id = first["id"].as_str().unwrap();
    let second_id = second["id"].as_str().unwrap();

    let (status, a) = get_fingerprint(&base, &home.token, first_id).await;
    assert_eq!(status, StatusCode::OK, "{a}");
    assert!(a["duplicate_of"].is_null(), "{a}");
    assert_eq!(a["fingerprint"].as_str().unwrap().len(), 64);
    let (status, b) = get_fingerprint(&base, &home.token, second_id).await;
    assert_eq!(status, StatusCode::OK, "{b}");
    assert_eq!(b["fingerprint"], a["fingerprint"]);
    assert_eq!(b["duplicate_of"], json!(first_id));

    for id in [first_id, second_id] {
        let (status, body) = get_fingerprint(&base, &away.token, id).await;
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
            "{status} {body}"
        );
        assert!(body.get("fingerprint").is_none(), "{body}");
    }
}

/// Within one workspace, a duplicate can point at a task in a private
/// channel: a caller outside that channel reads the duplicate's fingerprint
/// without learning the original exists, and a member of it sees the link.
#[tokio::test]
async fn duplicate_of_is_hidden_unless_the_caller_can_see_the_original() {
    let (base, store) = spawn().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "private-original".into(),
        })
        .await
        .unwrap();
    let (alice, alice_token) = member_with(store.as_ref(), ws.id, "alice", CAPS).await;
    let (_bob, bob_token) = member_with(store.as_ref(), ws.id, "bob", CAPS).await;
    let mut channels = Vec::new();
    for (name, private) in [("secret", true), ("open", false)] {
        channels.push(
            store
                .create_channel(NewChannel {
                    workspace_id: ws.id,
                    name: name.into(),
                    topic: None,
                    private,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let (secret, open) = (channels[0], channels[1]);
    store
        .add_channel_member(secret, alice, ChannelMemberRole::Member)
        .await
        .unwrap();
    let mut ids = Vec::new();
    for channel in [secret, open] {
        let (thread, _event) = store
            .create_thread_with_footprint(
                maidan_types::NewThread {
                    channel_id: channel,
                    parent_thread_id: None,
                    title: Some("Rotate the keys".into()),
                    description: None,
                    created_by: Some(alice),
                },
                None,
                None,
                None,
            )
            .await
            .unwrap();
        ids.push(thread.id.0.to_string());
    }
    let (original, duplicate) = (&ids[0], &ids[1]);

    // Bob cannot read the original at all.
    let (status, _) = get_fingerprint(&base, &bob_token, original).await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
        "{status}"
    );
    // Bob reads the duplicate, but not where it points.
    let (status, body) = get_fingerprint(&base, &bob_token, duplicate).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["fingerprint"].as_str().unwrap().len(), 64);
    assert!(body["duplicate_of"].is_null(), "{body}");
    // Alice can see the original, so she sees the link.
    let (status, body) = get_fingerprint(&base, &alice_token, duplicate).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["duplicate_of"], json!(original));

    // Bob cannot clear a link to an original he cannot see: he gets the
    // same 404 a missing thread gives, and the link stays (#1580).
    let http = reqwest::Client::new();
    let clear = |thread: String, token: String| {
        let http = http.clone();
        let base = base.clone();
        async move {
            let r = http
                .delete(format!("{base}/threads/{thread}/fingerprint/duplicate-of"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap();
            (r.status(), r.text().await.unwrap())
        }
    };
    let (status, body) = clear(duplicate.clone(), bob_token.clone()).await;
    let (missing_status, _) = clear(uuid::Uuid::new_v4().to_string(), bob_token.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(status, missing_status);
    let (status, body) = get_fingerprint(&base, &alice_token, duplicate).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["duplicate_of"], json!(original));
    // Alice can see both, so she can clear it.
    let (status, body) = clear(duplicate.clone(), alice_token.clone()).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

async fn claim_next_text(base: &str, room: &Room) -> String {
    reqwest::Client::new()
        .post(format!(
            "{base}/channels/{}/threads/claim-next",
            room.channel.0
        ))
        .bearer_auth(&room.token)
        .json(&json!({ "lease_secs": 600 }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

/// A live duplicate is skipped by claim-next until its link is cleared
/// (C8b); clearing needs `thread:transition` and a fingerprint.
#[tokio::test]
async fn claim_next_skips_a_live_duplicate_until_its_link_is_cleared() {
    let (base, store) = spawn().await;
    let home = room(store.as_ref(), "dupe-queue").await;
    let brief = json!({ "title": "Fix the sweep" });
    let (_, original) = create_thread(&base, &home, brief.clone()).await;
    let (_, duplicate) = create_thread(&base, &home, brief).await;
    let original = original["id"].as_str().unwrap().to_string();
    let duplicate = duplicate["id"].as_str().unwrap().to_string();

    let first = claim_next_text(&base, &home).await;
    assert!(first.contains(&original), "{first}");
    let second = claim_next_text(&base, &home).await;
    assert!(!second.contains(&duplicate), "{second}");

    let http = reqwest::Client::new();
    let url = format!("{base}/threads/{duplicate}/fingerprint/duplicate-of");
    // Another tenant cannot clear it, and gets exactly what a missing thread
    // gets. Checked before the real clear, so it is a refusal, not a no-op.
    let away = room(store.as_ref(), "dupe-away").await;
    let delete = |url: String, token: String| {
        let http = http.clone();
        async move {
            let r = http.delete(url).bearer_auth(token).send().await.unwrap();
            (r.status(), r.text().await.unwrap())
        }
    };
    let missing = format!(
        "{base}/threads/{}/fingerprint/duplicate-of",
        uuid::Uuid::new_v4()
    );
    let (away_status, _) = delete(url.clone(), away.token.clone()).await;
    let (missing_status, _) = delete(missing, away.token.clone()).await;
    assert_eq!(away_status, StatusCode::NOT_FOUND);
    assert_eq!(away_status, missing_status);
    let still = claim_next_text(&base, &home).await;
    assert!(!still.contains(&duplicate), "{still}");

    let (status, body) = delete(url.clone(), home.token.clone()).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let third = claim_next_text(&base, &home).await;
    assert!(third.contains(&duplicate), "{third}");

    // The clear is audited in the home workspace.
    let ws = store.get_channel(home.channel).await.unwrap().workspace_id;
    let rows = store.list_audit_for_workspace(ws, 50).await.unwrap();
    assert!(
        rows.iter().any(|r| r.action == "thread.duplicate_cleared"
            && r.target_id.map(|t| t.to_string()) == Some(duplicate.clone())),
        "{rows:?}"
    );
}

async fn get_json(base: &str, token: &str, path: &str) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

/// A task withheld as a duplicate says why in its create answer, on its own
/// read and in queue depth, so a claimer is not left with an open thread
/// that never comes; clearing the link clears the reason, and another tenant
/// learns nothing (#1681).
#[tokio::test]
async fn a_withheld_duplicate_names_why_in_the_create_answer_the_thread_read_and_queue_depth() {
    let (base, store) = spawn().await;
    let home = room(store.as_ref(), "withheld-home").await;
    let away = room(store.as_ref(), "withheld-away").await;
    let brief = json!({ "title": "Replay review 1" });
    let (_, original) = create_thread(&base, &home, brief.clone()).await;
    let (status, duplicate) = create_thread(&base, &home, brief).await;
    assert_eq!(status, StatusCode::CREATED, "{duplicate}");
    let original_id = original["id"].as_str().unwrap().to_string();
    let duplicate_id = duplicate["id"].as_str().unwrap().to_string();

    assert!(original.get("withheld").is_none(), "{original}");
    assert_eq!(
        duplicate["withheld"],
        json!({ "reason": "duplicate_of", "thread": original_id })
    );

    let (status, read) = get_json(&base, &home.token, &format!("/threads/{duplicate_id}")).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["withheld"]["reason"], json!("duplicate_of"));
    assert_eq!(read["withheld"]["thread"], json!(original_id));
    let (_, read) = get_json(&base, &home.token, &format!("/threads/{original_id}")).await;
    assert!(read.get("withheld").is_none(), "{read}");

    let (status, depth) = get_json(
        &base,
        &home.token,
        &format!("/channels/{}/queue-depth", home.channel.0),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{depth}");
    assert_eq!(depth["unclaimable"], json!(1), "{depth}");

    let (status, body) = get_json(&base, &away.token, &format!("/threads/{duplicate_id}")).await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
        "{status} {body}"
    );
    assert!(body.get("withheld").is_none(), "{body}");

    let status = reqwest::Client::new()
        .delete(format!(
            "{base}/threads/{duplicate_id}/fingerprint/duplicate-of"
        ))
        .bearer_auth(&home.token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, read) = get_json(&base, &home.token, &format!("/threads/{duplicate_id}")).await;
    assert!(read.get("withheld").is_none(), "{read}");
}

async fn mcp_thread(base: &str, token: &str, thread: &str) -> Value {
    let read: Value = reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "resources/read",
            "params": { "uri": format!("maidan://threads/{thread}") }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = read["result"]["contents"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{read}"));
    serde_json::from_str(text).unwrap()
}

/// A duplicate withheld at creation carries a notice on its own thread, and
/// the MCP thread read names why it is withheld just as the REST read does,
/// while the original gets neither (#1681).
#[tokio::test]
async fn a_withheld_duplicate_posts_a_thread_notice_and_says_why_on_the_mcp_read() {
    let (base, store) = spawn().await;
    let home = room(store.as_ref(), "notice-home").await;
    let brief = json!({ "title": "Replay review 2" });
    let (_, original) = create_thread(&base, &home, brief.clone()).await;
    let (status, duplicate) = create_thread(&base, &home, brief).await;
    assert_eq!(status, StatusCode::CREATED, "{duplicate}");
    let original_id = original["id"].as_str().unwrap().to_string();
    let duplicate_id = duplicate["id"].as_str().unwrap().to_string();

    let (status, messages) = get_json(
        &base,
        &home.token,
        &format!("/threads/{duplicate_id}/messages"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{messages}");
    let notices: Vec<&Value> = messages
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["metadata"]["notice"] == json!("withheld"))
        .collect();
    assert_eq!(notices.len(), 1, "{messages}");
    assert_eq!(notices[0]["metadata"]["duplicate_of"], json!(original_id));
    assert!(
        notices[0]["body"].as_str().unwrap().contains(&original_id),
        "{messages}"
    );
    let (_, messages) = get_json(
        &base,
        &home.token,
        &format!("/threads/{original_id}/messages"),
    )
    .await;
    assert!(
        !messages
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["metadata"]["notice"] == json!("withheld")),
        "{messages}"
    );

    let via_mcp = mcp_thread(&base, &home.token, &duplicate_id).await;
    assert_eq!(
        via_mcp["thread"]["withheld"],
        json!({ "reason": "duplicate_of", "thread": original_id })
    );
    let via_mcp = mcp_thread(&base, &home.token, &original_id).await;
    assert!(via_mcp["thread"].get("withheld").is_none(), "{via_mcp}");
}
