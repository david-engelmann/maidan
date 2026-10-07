//! The dev instance's anonymous MCP reader, over HTTP with auth enabled. A
//! `POST` to an MCP endpoint with no credential reads the one synthetic
//! workspace and nothing else: only read-only tools, each marked as needing no
//! sign-in, no write even where `workspace:read` would be enough, no
//! subscription, no session, and no other workspace. Every other route and
//! method still needs a credential, and a bearer caller is unaffected.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{dev_anonymous, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct World {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    synthetic: maidan_types::WorkspaceId,
    other: maidan_types::WorkspaceId,
    bearer: String,
}

async fn spawn() -> World {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));

    let synthetic = store
        .create_workspace(NewWorkspace {
            name: "synthetic-demo".into(),
        })
        .await
        .unwrap();
    store
        .create_channel(NewChannel {
            workspace_id: synthetic.id,
            name: "fixtures".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let other = store
        .create_workspace(NewWorkspace {
            name: "team".into(),
        })
        .await
        .unwrap();
    store
        .create_channel(NewChannel {
            workspace_id: other.id,
            name: "payroll".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let human = store
        .create_member(NewMember {
            workspace_id: other.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: other.id,
            member_id: human.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();

    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
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
    state.dev_anonymous_reader = Some(
        dev_anonymous::reader_for(store.as_ref(), synthetic.id)
            .await
            .unwrap(),
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    World {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        store,
        synthetic: synthetic.id,
        other: other.id,
        bearer: secret.as_str().to_string(),
    }
}

impl World {
    async fn rpc(&self, path: &str, bearer: Option<&str>, method: &str, params: Value) -> Value {
        let mut request = self
            .client
            .post(format!("{}{path}", self.base))
            .header("Accept", "application/json")
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{method} on {path}");
        response.json().await.unwrap()
    }

    async fn call(&self, tool: &str, arguments: Value) -> Value {
        self.rpc(
            "/mcp",
            None,
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        )
        .await
    }
}

fn refused(response: &Value) -> bool {
    response["error"]["message"]
        .as_str()
        .is_some_and(|m| m.contains("bearer token"))
}

#[tokio::test]
async fn a_caller_with_no_credential_reads_the_synthetic_workspace_and_nothing_else() {
    let world = spawn().await;

    for path in ["/mcp", "/mcp/streamable"] {
        let listed = world.rpc(path, None, "tools/list", json!({})).await;
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert!(!tools.is_empty(), "{path}: {listed}");
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            assert!(
                maidan_mcp::tools::is_read_only(name),
                "{path} lists {name}, which writes"
            );
            assert_eq!(
                tool["securitySchemes"],
                json!([{ "type": "noauth" }]),
                "{name}"
            );
        }
    }

    let channels = world
        .call("list_channels", json!({ "workspace_id": world.synthetic }))
        .await;
    let text = channels["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("fixtures"), "{channels}");

    let elsewhere = world
        .call("list_channels", json!({ "workspace_id": world.other }))
        .await;
    assert!(
        !elsewhere.to_string().contains("payroll"),
        "another workspace leaked: {elsewhere}"
    );

    let write = world
        .call(
            "create_channel",
            json!({ "workspace_id": world.synthetic, "name": "nope" }),
        )
        .await;
    assert!(refused(&write), "{write}");
    let read_cap_write = world
        .call(
            "open_group_dm",
            json!({ "workspace_id": world.synthetic, "member_ids": [] }),
        )
        .await;
    assert!(
        refused(&read_cap_write),
        "a write that asks only workspace:read is still a write: {read_cap_write}"
    );
    let subscribe = world
        .rpc(
            "/mcp",
            None,
            "resources/subscribe",
            json!({ "uri": format!("maidan://workspace/{}", world.synthetic) }),
        )
        .await;
    assert!(refused(&subscribe), "{subscribe}");
    let channels_after = world.store.list_channels(world.synthetic).await.unwrap();
    assert_eq!(channels_after.len(), 1, "nothing was created");
}

#[tokio::test]
async fn only_an_mcp_post_is_open_and_never_a_session() {
    let world = spawn().await;

    let listener = world
        .client
        .get(format!("{}/mcp/streamable", world.base))
        .header("Accept", "text/event-stream")
        .send()
        .await
        .unwrap();
    assert_eq!(listener.status(), StatusCode::UNAUTHORIZED);
    let rest = world
        .client
        .get(format!(
            "{}/workspaces/{}/channels",
            world.base, world.synthetic
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(rest.status(), StatusCode::UNAUTHORIZED);

    let legacy = world
        .client
        .post(format!("{}/mcp/streamable", world.base))
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "dev", "version": "0" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::OK);
    assert!(
        legacy.headers().get("mcp-session-id").is_none(),
        "every anonymous caller is one member, so none gets a session"
    );
}

#[tokio::test]
async fn a_bearer_caller_is_unchanged() {
    let world = spawn().await;
    let bearer = world.bearer.clone();
    let listed = world
        .rpc("/mcp", Some(&bearer), "tools/list", json!({}))
        .await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert!(tools
        .iter()
        .any(|t| !maidan_mcp::tools::is_read_only(t["name"].as_str().unwrap())));
    assert!(tools.iter().all(|t| t.get("securitySchemes").is_none()));
}

#[tokio::test]
async fn only_a_workspace_named_synthetic_is_read_without_a_credential() {
    let world = spawn().await;
    let refused = dev_anonymous::reader_for(world.store.as_ref(), world.other).await;
    assert!(refused.is_err());
}
