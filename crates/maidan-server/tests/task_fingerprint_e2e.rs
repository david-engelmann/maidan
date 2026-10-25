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
    let status = http
        .delete(&url)
        .bearer_auth(&home.token)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::NO_CONTENT);
    let third = claim_next_text(&base, &home).await;
    assert!(third.contains(&duplicate), "{third}");

    // Another tenant cannot clear it.
    let away = room(store.as_ref(), "dupe-away").await;
    let status = http
        .delete(&url)
        .bearer_auth(&away.token)
        .send()
        .await
        .unwrap()
        .status();
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
        "{status}"
    );
}
