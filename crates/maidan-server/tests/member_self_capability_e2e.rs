//! MCP-002: writes to a member's own personal state need `member:self`, and a
//! token minted before that capability existed keeps them through migration
//! 0237, which grants `member:self` to every stored list holding
//! `workspace:read`.
//!
//! The token here is made the way a live one was: its capabilities written as
//! they stood before the change, then the migration run over the row. Before
//! the migration the new gate refuses it; after, it marks a notification read
//! over MCP and REST as it did before this change.

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
    ApiTokenId, EventKind, MemberKind, NewApiToken, NewChannel, NewMember, NewNotification,
    NewThread, NewWorkspace,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

const SQLITE_0237: &str =
    include_str!("../../../migrations/sqlite/0237_member_self_capability.sql");

/// The worker preset as it stood before `member:self`: what a seat or a
/// coordinator token holds today.
const PRE_CHANGE_WORKER: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::MESSAGE_POST,
    capability::EVENT_SUBSCRIBE,
    capability::SEARCH_QUERY,
    capability::ARTIFACT_UPLOAD,
    capability::THREAD_TRANSITION,
];

struct World {
    base: String,
    client: reqwest::Client,
    pool: sqlx::SqlitePool,
    store: Arc<dyn Store>,
}

async fn spawn() -> World {
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
    let state = AppState::new(
        store.clone(),
        artifacts,
        Arc::new(InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    World {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        pool,
        store,
    }
}

impl World {
    async fn call(&self, bearer: &str, tool: &str, arguments: Value) -> Value {
        let response = self
            .client
            .post(format!("{}/mcp", self.base))
            .header("Accept", "application/json")
            .bearer_auth(bearer)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": tool, "arguments": arguments },
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{tool}");
        response.json().await.unwrap()
    }

    async fn post(&self, bearer: &str, path: &str) -> StatusCode {
        self.client
            .post(format!("{}{path}", self.base))
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn stored_caps(&self, id: ApiTokenId) -> Vec<String> {
        self.store.get_api_token(id).await.unwrap().capabilities
    }
}

fn missing_member_self(response: &Value) -> bool {
    response["error"]["message"]
        .as_str()
        .is_some_and(|m| m.contains("missing capability: member:self"))
}

#[tokio::test]
async fn a_token_minted_before_member_self_still_marks_a_notification_read_after_the_migration() {
    let world = spawn().await;
    let store = world.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace {
            name: "member-self".into(),
        })
        .await
        .unwrap();
    let seat = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "seat".into(),
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
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let mut notifications = Vec::new();
    for log_id in 1..=2 {
        notifications.push(
            store
                .create_notification(NewNotification {
                    workspace_id: ws.id,
                    member_id: seat.id,
                    kind: EventKind::MentionRecorded,
                    source_log_id: log_id,
                    channel_id: Some(channel.id),
                    thread_id: Some(thread.id),
                    message_id: None,
                    actor_id: None,
                })
                .await
                .unwrap()
                .id,
        );
    }

    // A live token: written with the capabilities it was minted with before
    // this change, so it holds `workspace:read` and no `member:self`.
    let secret = TokenSecret::generate();
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: seat.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("seat".into()),
            capabilities: PRE_CHANGE_WORKER.iter().map(|c| c.to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    let bearer = secret.as_str();
    // A token that holds only `workspace:read` stays read-only: the migration
    // grants it `member:self` too, since it could make these writes before.
    let read_secret = TokenSecret::generate();
    let read_only = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: seat.id,
            app_installation_id: None,
            token_hash: hash_secret(read_secret.as_str()),
            label: Some("reader".into()),
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    // One minted for something else entirely is not touched.
    let search_secret = TokenSecret::generate();
    let search_only = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: seat.id,
            app_installation_id: None,
            token_hash: hash_secret(search_secret.as_str()),
            label: Some("search".into()),
            capabilities: vec![capability::SEARCH_QUERY.into()],
            expires_at: None,
        })
        .await
        .unwrap();

    // Before the data fix, the new gate refuses the old token: this is the
    // break the migration exists to prevent.
    let refused = world
        .call(
            bearer,
            "mark_notification_read",
            json!({ "member_id": seat.id, "notification_id": notifications[0] }),
        )
        .await;
    assert!(missing_member_self(&refused), "{refused}");
    assert_eq!(
        world
            .post(
                bearer,
                &format!(
                    "/members/{}/notifications/{}/read",
                    seat.id, notifications[1]
                ),
            )
            .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        store
            .list_notifications(seat.id, true, 10)
            .await
            .unwrap()
            .len(),
        2,
        "nothing was marked read while refused"
    );

    sqlx::raw_sql(SQLITE_0237)
        .execute(&world.pool)
        .await
        .unwrap();

    let mut expected: Vec<String> = PRE_CHANGE_WORKER.iter().map(|c| c.to_string()).collect();
    expected.push(capability::MEMBER_SELF.into());
    assert_eq!(
        world.stored_caps(token.id).await,
        expected,
        "appended, order kept"
    );
    assert_eq!(
        world.stored_caps(read_only.id).await,
        vec![capability::WORKSPACE_READ, capability::MEMBER_SELF]
    );
    assert_eq!(
        world.stored_caps(search_only.id).await,
        vec![capability::SEARCH_QUERY],
        "no workspace:read, so no member:self"
    );

    // The same token, unchanged on the caller's side, marks read again.
    let marked = world
        .call(
            bearer,
            "mark_notification_read",
            json!({ "member_id": seat.id, "notification_id": notifications[0] }),
        )
        .await;
    assert!(marked["error"].is_null(), "{marked}");
    assert_eq!(
        world
            .post(
                bearer,
                &format!(
                    "/members/{}/notifications/{}/read",
                    seat.id, notifications[1]
                ),
            )
            .await,
        StatusCode::OK
    );
    assert!(
        store
            .list_notifications(seat.id, true, 10)
            .await
            .unwrap()
            .is_empty(),
        "both marked read"
    );
    for (tool, args) in [
        (
            "mark_inbox_read",
            json!({ "member_id": seat.id, "read_through": "2026-11-11T00:00:00Z" }),
        ),
        (
            "follow_thread",
            json!({ "member_id": seat.id, "thread_id": thread.id }),
        ),
        (
            "unfollow_thread",
            json!({ "member_id": seat.id, "thread_id": thread.id }),
        ),
    ] {
        let response = world.call(bearer, tool, args).await;
        assert!(response["error"].is_null(), "{tool}: {response}");
    }

    // Run again, nothing changes: no second `member:self`.
    sqlx::raw_sql(SQLITE_0237)
        .execute(&world.pool)
        .await
        .unwrap();
    assert_eq!(world.stored_caps(token.id).await, expected);
}

#[tokio::test]
async fn a_bare_workspace_read_token_minted_now_cannot_write_personal_state() {
    let world = spawn().await;
    let store = world.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace {
            name: "read-only".into(),
        })
        .await
        .unwrap();
    let reader = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "reader".into(),
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
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: reader.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let bearer = secret.as_str();

    for name in maidan_mcp::tools::MEMBER_SELF_TOOLS {
        let response = world
            .call(
                bearer,
                name,
                json!({ "member_id": reader.id, "channel_id": channel.id }),
            )
            .await;
        assert!(missing_member_self(&response), "{name}: {response}");
    }
    assert_eq!(
        world
            .post(
                bearer,
                &format!("/members/{}/notifications/read-all", reader.id)
            )
            .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        world
            .post(bearer, &format!("/channels/{}/mute", channel.id))
            .await,
        StatusCode::FORBIDDEN
    );
    assert!(
        store
            .list_channel_follows(reader.id)
            .await
            .unwrap()
            .is_empty(),
        "nothing written"
    );
}
