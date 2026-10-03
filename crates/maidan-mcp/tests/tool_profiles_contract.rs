//! Worker and reviewer profiles: a fixed `tools/list` on its own endpoint,
//! the same bytes for every caller, with a tool the token cannot call refused
//! when it is called.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_mcp::caching::{self, CacheScope};
use maidan_mcp::{JsonRpcRequest, JsonRpcResponse, McpServer, Profile, INSTRUCTIONS};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mk_server() -> (McpServer, Arc<dyn Store>) {
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
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    (server, store)
}

async fn tenant(store: &Arc<dyn Store>, name: &str) -> (WorkspaceId, MemberId) {
    let workspace = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: format!("{name}-agent"),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    (workspace.id, member.id)
}

fn token(workspace: WorkspaceId, member: MemberId, capabilities: &[&str]) -> AuthContext {
    AuthContext::from_token(
        ApiTokenId(uuid::Uuid::now_v7()),
        member,
        workspace,
        capabilities.iter().map(|c| c.to_string()).collect(),
    )
}

fn request(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: method.into(),
        params,
    }
}

async fn profile_call(
    server: &McpServer,
    auth: &AuthContext,
    profile: Profile,
    method: &str,
    params: Value,
) -> JsonRpcResponse {
    server
        .handle_profile(request(method, params), auth, profile)
        .await
}

fn golden(profile: Profile) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("profile-{}.json", profile.name()));
    std::fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

#[test]
fn instructions_fit_the_cold_start_limits() {
    let first = INSTRUCTIONS.lines().next().unwrap_or("");
    assert!(
        first.chars().count() <= 250,
        "first line is {} characters",
        first.chars().count()
    );
    assert!(
        INSTRUCTIONS.chars().count() <= 2048,
        "instructions are {} characters",
        INSTRUCTIONS.chars().count()
    );
    assert!(first.contains("whoami"), "{first}");
    assert!(first.contains("claim_next_thread"), "{first}");
    assert!(first.contains("release_claim"), "{first}");
}

#[tokio::test]
async fn a_profile_list_is_public_sorted_and_the_same_bytes_for_two_tenants() {
    let (server, store) = mk_server().await;
    let (workspace_a, member_a) = tenant(&store, "alpha").await;
    let (workspace_b, member_b) = tenant(&store, "beta").await;
    let reader = token(workspace_a, member_a, &[WORKSPACE_READ]);
    let worker = token(workspace_b, member_b, &[WORKSPACE_READ, THREAD_TRANSITION]);

    for profile in Profile::ALL {
        let once = profile_call(&server, &reader, profile, "tools/list", json!({}))
            .await
            .result
            .expect("tools/list");
        let twice = profile_call(&server, &reader, profile, "tools/list", json!({}))
            .await
            .result
            .expect("tools/list");
        let other = profile_call(&server, &worker, profile, "tools/list", json!({}))
            .await
            .result
            .expect("tools/list");
        assert_eq!(once["cacheScope"], CacheScope::Public.as_str());
        assert_eq!(once["ttlMs"], json!(caching::RELEASE_TTL_MS));
        assert_eq!(once["resultType"], "complete");
        let bytes = serde_json::to_vec(&once).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&twice).unwrap(), "call");
        assert_eq!(bytes, serde_json::to_vec(&other).unwrap(), "tenant");
        assert_eq!(
            serde_json::to_vec(&once["tools"]).unwrap(),
            golden(profile),
            "{}",
            profile.name()
        );
        let names: Vec<&str> = once["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert_eq!(names, profile.tool_names());
        // The reader cannot call a transition tool, and the list still names it.
        assert!(names.contains(&"transition_thread"), "{}", profile.name());
    }

    let full = server
        .handle(request("tools/list", json!({})), &reader)
        .await
        .result
        .unwrap();
    assert_eq!(full["cacheScope"], "private");
    let full_names: Vec<&str> = full["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(
        !full_names.contains(&"transition_thread"),
        "the full catalog stays filtered to the token"
    );
    assert!(full_names.len() > Profile::Worker.tool_names().len());
}

#[tokio::test]
async fn a_profile_refuses_a_tool_the_token_cannot_call_and_one_it_does_not_list() {
    let (server, store) = mk_server().await;
    let (workspace_a, member_a) = tenant(&store, "alpha").await;
    let (workspace_b, member_b) = tenant(&store, "beta").await;
    let reader = token(workspace_a, member_a, &[WORKSPACE_READ]);
    let other = token(workspace_b, member_b, &[WORKSPACE_READ]);

    let missing = profile_call(
        &server,
        &reader,
        Profile::Worker,
        "tools/call",
        json!({ "name": "claim_next_thread", "arguments": {} }),
    )
    .await;
    let err = missing.error.expect("capability refusal");
    assert_eq!(err.code, -32003);
    assert!(
        err.message.contains("missing capability"),
        "{}",
        err.message
    );

    let outside = profile_call(
        &server,
        &reader,
        Profile::Worker,
        "tools/call",
        json!({ "name": "list_channels", "arguments": {} }),
    )
    .await;
    let err = outside.error.expect("profile refusal");
    assert_eq!(err.code, -32003);
    assert!(
        err.message.contains("not on the worker profile"),
        "{}",
        err.message
    );

    for (auth, workspace, foreign) in [
        (&reader, workspace_a, workspace_b),
        (&other, workspace_b, workspace_a),
    ] {
        let who = profile_call(
            &server,
            auth,
            Profile::Worker,
            "tools/call",
            json!({ "name": "whoami", "arguments": {} }),
        )
        .await
        .result
        .expect("whoami");
        let text = who["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        assert_eq!(body["workspace_id"], json!(workspace.0));
        assert_ne!(body["workspace_id"], json!(foreign.0));
    }
}
