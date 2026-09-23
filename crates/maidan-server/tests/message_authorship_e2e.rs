//! A message says who wrote it, so only its author may change what it says.
//!
//! Three operations touch a message's words after it is posted, and each sits
//! at its own bar:
//!
//! - **edit** — the author only. The message stays under the author's name,
//!   so anyone else's edit would record them saying what they did not. There
//!   is no moderator override: moderation removes, it never rewrites.
//! - **tombstone** — the author, as part of posting; anyone else needs
//!   `channel:admin`, because blanking another member's words is moderation.
//! - **purge** — `token:admin`, the bar for destroying the record.
//!
//! Each of these used to accept `workspace:write`, which every agent holds and
//! any delegation grant can lend.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const MCP_FORBIDDEN: i64 = -32003;

/// Every capability an ordinary agent is minted with.
const AGENT: &[&str] = &[
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
    thread: ThreadId,
    /// The author: nothing beyond reading and posting.
    alice: String,
    /// Another agent in the same room, with every work capability.
    bob: String,
    /// Moderation authority over channels.
    moderator: String,
    /// Workspace authority.
    admin: String,
    _dir: tempfile::TempDir,
}

async fn world() -> World {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
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
        })
        .await
        .unwrap();

    let mut tokens = Vec::new();
    for (handle, caps) in [
        (
            "alice",
            vec![capability::WORKSPACE_READ, capability::MESSAGE_POST],
        ),
        ("bob", AGENT.to_vec()),
        (
            "moderator",
            vec![
                capability::WORKSPACE_READ,
                capability::MESSAGE_POST,
                capability::CHANNEL_ADMIN,
            ],
        ),
        (
            "admin",
            vec![
                capability::WORKSPACE_READ,
                capability::MESSAGE_POST,
                capability::TOKEN_ADMIN,
            ],
        ),
    ] {
        let member: MemberId = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap()
            .id;
        let secret = TokenSecret::generate();
        store
            .create_api_token(NewApiToken {
                workspace_id: ws.id,
                member_id: member,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: Some(handle.into()),
                capabilities: caps.into_iter().map(String::from).collect(),
                expires_at: None,
            })
            .await
            .unwrap();
        tokens.push(secret.as_str().to_string());
    }

    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });

    let mut tokens = tokens.into_iter();
    let mut next = || tokens.next().unwrap();
    World {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        thread: thread.id,
        alice: next(),
        bob: next(),
        moderator: next(),
        admin: next(),
        _dir: dir,
    }
}

impl World {
    async fn post(&self, token: &str, body: &str) -> String {
        let resp = self
            .client
            .post(format!("{}/threads/{}/messages", self.base, self.thread.0))
            .bearer_auth(token)
            .json(&json!({ "body": body }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let msg: Value = resp.json().await.unwrap();
        msg["id"].as_str().unwrap().to_string()
    }

    async fn edit(&self, token: &str, id: &str, body: &str) -> StatusCode {
        self.client
            .patch(format!("{}/messages/{id}", self.base))
            .bearer_auth(token)
            .json(&json!({ "body": body }))
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn tombstone(&self, token: &str, id: &str) -> StatusCode {
        self.client
            .delete(format!("{}/messages/{id}", self.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn purge(&self, token: &str, id: &str) -> StatusCode {
        self.client
            .delete(format!("{}/messages/{id}/purge", self.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn body(&self, id: &str) -> String {
        let msg: Value = self
            .client
            .get(format!("{}/messages/{id}", self.base))
            .bearer_auth(&self.admin)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        msg["body"].as_str().unwrap_or_default().to_string()
    }
}

#[tokio::test]
async fn no_one_but_the_author_can_edit_a_message() {
    let w = world().await;
    let id = w.post(&w.alice, "ship it friday").await;

    for (who, token) in [
        ("an agent with every work capability", &w.bob),
        ("a channel moderator", &w.moderator),
        ("a workspace admin", &w.admin),
    ] {
        assert_eq!(
            w.edit(token, &id, "do not ship").await,
            StatusCode::FORBIDDEN,
            "{who} must not be able to rewrite alice's message"
        );
    }
    assert_eq!(w.body(&id).await, "ship it friday");

    assert_eq!(
        w.edit(&w.alice, &id, "ship it monday").await,
        StatusCode::OK
    );
    assert_eq!(w.body(&id).await, "ship it monday");
}

#[tokio::test]
async fn mcp_edit_is_author_only_like_rest() {
    let w = world().await;
    let id = w.post(&w.alice, "ship it friday").await;

    let call = |token: &str| {
        w.client
            .post(format!("{}/mcp", w.base))
            .bearer_auth(token)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "edit_message",
                    "arguments": { "message_id": id, "body": "do not ship" }
                }
            }))
            .send()
    };
    let resp: Value = call(&w.bob).await.unwrap().json().await.unwrap();
    assert_eq!(
        resp["error"]["code"].as_i64(),
        Some(MCP_FORBIDDEN),
        "another agent must not rewrite alice's message over MCP: {resp}"
    );
    assert_eq!(w.body(&id).await, "ship it friday");

    let resp: Value = call(&w.alice).await.unwrap().json().await.unwrap();
    assert!(resp["error"].is_null(), "the author can edit: {resp}");
    assert_eq!(w.body(&id).await, "do not ship");
}

#[tokio::test]
async fn blanking_someone_elses_message_is_moderation() {
    let w = world().await;
    let id = w.post(&w.alice, "the numbers are wrong").await;

    assert_eq!(
        w.tombstone(&w.bob, &id).await,
        StatusCode::FORBIDDEN,
        "another agent in the room must not be able to erase what alice said"
    );
    assert_eq!(w.body(&id).await, "the numbers are wrong");

    assert_eq!(w.tombstone(&w.moderator, &id).await, StatusCode::NO_CONTENT);
    assert_eq!(w.body(&id).await, "");
}

#[tokio::test]
async fn an_author_can_withdraw_their_own_message() {
    let w = world().await;
    // alice holds only read + post: withdrawing her own words needs nothing more.
    let id = w.post(&w.alice, "wrong thread").await;
    assert_eq!(w.tombstone(&w.alice, &id).await, StatusCode::NO_CONTENT);
    assert_eq!(w.body(&id).await, "");
}

#[tokio::test]
async fn purging_a_message_takes_workspace_authority() {
    let w = world().await;
    let id = w.post(&w.alice, "leaked key").await;
    // Purge hard-deletes a message that is already tombstoned. Withdrawing it
    // first leaves the capability as the only thing a purge can be refused on.
    assert_eq!(w.tombstone(&w.alice, &id).await, StatusCode::NO_CONTENT);

    for (who, token) in [
        ("an agent with every work capability", &w.bob),
        ("a channel moderator", &w.moderator),
        ("the author", &w.alice),
    ] {
        assert_eq!(
            w.purge(token, &id).await,
            StatusCode::FORBIDDEN,
            "{who} must not be able to destroy the record"
        );
    }
    assert_eq!(w.purge(&w.admin, &id).await, StatusCode::NO_CONTENT);
}
