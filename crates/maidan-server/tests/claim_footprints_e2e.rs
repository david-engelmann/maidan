//! Claim footprints over the wire, as a swarm agent uses them: claim over MCP,
//! declare the footprint over REST or MCP, and get a 409 (REST) or error
//! -32009 (MCP) naming the overlapped claim and its lease when another agent
//! already holds the work. Another tenant reads none of it.

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
    ThreadId, WorkspaceId,
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

async fn agent(store: &dyn Store, ws: WorkspaceId, handle: &str) -> (MemberId, String) {
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
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    (member.id, secret.as_str().to_string())
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

async fn claim(base: &str, token: &str, channel: ChannelId) -> ThreadId {
    let claimed = tool_result(
        &tool(
            base,
            token,
            "claim_next_thread",
            json!({ "channel_id": channel.0, "lease_secs": 600 }),
        )
        .await,
    );
    let id = claimed["thread"]["id"]
        .as_str()
        .or_else(|| claimed["id"].as_str())
        .unwrap_or_else(|| panic!("no thread claimed: {claimed}"));
    ThreadId(id.parse().unwrap())
}

fn lane(value: &str) -> Value {
    json!({
        "repo": "https://github.com/Example/App.git",
        "mode": "exclusive",
        "items": [{ "kind": "lane", "value": value }]
    })
}

#[tokio::test]
async fn a_second_agent_on_held_work_is_told_who_holds_it_and_until_when() {
    let (base, store) = spawn().await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "swarm".into(),
        })
        .await
        .unwrap();
    let (first_agent, first) = agent(store.as_ref(), ws.id, "first").await;
    let (_, second) = agent(store.as_ref(), ws.id, "second").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    for title in ["one", "two"] {
        store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
            })
            .await
            .unwrap();
    }
    let http = reqwest::Client::new();

    let held = claim(&base, &first, channel.id).await;
    let put = http
        .put(format!("{base}/threads/{}/footprint", held.0))
        .bearer_auth(&first)
        .json(&lane("bridge"))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let stored: Value = put.json().await.unwrap();
    assert_eq!(stored["repo"], "example/app", "the clone URL is normalized");

    let mine = claim(&base, &second, channel.id).await;
    assert_ne!(mine, held);
    let refused = http
        .put(format!("{base}/threads/{}/footprint", mine.0))
        .bearer_auth(&second)
        .json(&lane("bridge"))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let problem: Value = refused.json().await.unwrap();
    assert_eq!(
        problem["type"],
        "https://maidan.dev/problems/footprint-conflict"
    );
    let conflict = &problem["conflicts"][0];
    assert_eq!(conflict["thread_id"], held.0.to_string());
    assert_eq!(conflict["assignee_id"], first_agent.0.to_string());
    assert_eq!(conflict["value"], "bridge");
    assert!(conflict["lease_expires_at"].is_string());

    // The same refusal over MCP is error -32009 with the same conflicts.
    let over_mcp = tool(
        &base,
        &second,
        "set_thread_footprint",
        json!({ "thread_id": mine.0, "repo": "example/app", "mode": "shared",
                "items": [{ "kind": "lane", "value": "bridge" }] }),
    )
    .await;
    assert_eq!(over_mcp["error"]["code"], -32009, "{over_mcp}");
    assert_eq!(
        over_mcp["error"]["data"]["conflicts"][0]["thread_id"],
        held.0.to_string()
    );

    // Both surfaces list the held claim, however the repo is spelled.
    let listed: Value = http
        .get(format!(
            "{base}/workspaces/{}/claims?repo=git@github.com:example/app.git",
            ws.id.0
        ))
        .bearer_auth(&second)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed[0]["thread_id"], held.0.to_string());
    let listed_mcp = tool_result(
        &tool(
            &base,
            &second,
            "list_active_claims",
            json!({ "workspace_id": ws.id.0, "repo": "example/app" }),
        )
        .await,
    );
    assert_eq!(listed_mcp, listed);

    // Another tenant reads none of it.
    let other = store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .unwrap();
    let (_, outsider) = agent(store.as_ref(), other.id, "outsider").await;
    let foreign = http
        .get(format!("{base}/workspaces/{}/claims", ws.id.0))
        .bearer_auth(&outsider)
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
    let foreign_thread = http
        .get(format!("{base}/threads/{}/footprint", held.0))
        .bearer_auth(&outsider)
        .send()
        .await
        .unwrap();
    assert!(
        matches!(
            foreign_thread.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ),
        "{}",
        foreign_thread.status()
    );
}
