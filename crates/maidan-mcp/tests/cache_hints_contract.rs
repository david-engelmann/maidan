//! MCP `2026-07-28` conformance for what a client may cache: `server/discover`
//! exists and carries the instructions, every cacheable result carries a
//! `ttlMs` and a `cacheScope` (SEP-2549), every result says `resultType`, and a
//! 2025 client's `initialize` is unchanged by any of it.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_mcp::caching::{self, CacheScope};
use maidan_mcp::{JsonRpcRequest, McpServer, INSTRUCTIONS, SUPPORTED_PROTOCOL_VERSIONS};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Tenant {
    workspace: WorkspaceId,
    member: MemberId,
    channel: ChannelId,
    thread: ThreadId,
}

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

async fn mk_tenant(store: &Arc<dyn Store>, name: &str) -> Tenant {
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
    let channel = store
        .create_channel(NewChannel {
            workspace_id: workspace.id,
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
            title: Some("task".into()),
        })
        .await
        .unwrap();
    Tenant {
        workspace: workspace.id,
        member: member.id,
        channel: channel.id,
        thread: thread.id,
    }
}

fn token(tenant: &Tenant, capabilities: &[&str]) -> AuthContext {
    AuthContext::from_token(
        ApiTokenId(uuid::Uuid::now_v7()),
        tenant.member,
        tenant.workspace,
        capabilities.iter().map(|c| c.to_string()).collect(),
    )
}

async fn call(server: &McpServer, auth: &AuthContext, method: &str, params: Value) -> Value {
    let response = server
        .handle(
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(1)),
                method: method.into(),
                params,
            },
            auth,
        )
        .await;
    assert!(
        response.error.is_none(),
        "{method} failed: {:?}",
        response.error
    );
    response.result.unwrap()
}

fn assert_hinted(method: &str, result: &Value) {
    let ttl = result["ttlMs"].as_u64();
    assert!(
        ttl.is_some(),
        "{method}: ttlMs must be a non-negative integer, got {}",
        result["ttlMs"]
    );
    assert!(
        matches!(result["cacheScope"].as_str(), Some("public" | "private")),
        "{method}: cacheScope must be \"public\" or \"private\", got {}",
        result["cacheScope"]
    );
}

async fn shared_artifact(store: &Arc<dyn Store>, tenants: &[&Tenant]) -> String {
    let sha = "c".repeat(64);
    store
        .upsert_artifact(NewArtifact {
            sha256: sha.clone(),
            size_bytes: 3,
            mime_type: Some("text/plain".into()),
            filename: None,
            kind: ArtifactKind::Attachment,
            uploaded_by: None,
        })
        .await
        .unwrap();
    for tenant in tenants {
        store
            .record_artifact_ref(tenant.workspace, &sha)
            .await
            .unwrap();
    }
    sha
}

#[tokio::test]
async fn every_cacheable_result_carries_a_ttl_and_a_cache_scope() {
    let (server, store) = mk_server().await;
    let tenant = mk_tenant(&store, "hinted").await;
    let sha = shared_artifact(&store, &[&tenant]).await;
    let auth = token(&tenant, &[WORKSPACE_READ]);

    for method in [
        "server/discover",
        "tools/list",
        "prompts/list",
        "resources/list",
        "resources/templates/list",
    ] {
        assert_hinted(method, &call(&server, &auth, method, json!({})).await);
    }
    for uri in [
        format!("maidan://workspaces/{}", tenant.workspace.0),
        format!("maidan://channels/{}", tenant.channel.0),
        format!("maidan://threads/{}", tenant.thread.0),
        format!("maidan://artifacts/{sha}"),
    ] {
        let read = call(&server, &auth, "resources/read", json!({ "uri": uri })).await;
        assert_hinted(&uri, &read);
        assert_eq!(
            read["ttlMs"].as_u64(),
            Some(caching::resource_read(&uri).ttl_ms),
            "{uri}"
        );
        assert_eq!(read["cacheScope"], "private", "{uri}: reads are per caller");
    }
}

#[tokio::test]
async fn an_artifact_read_uses_the_record_ttl_and_a_thread_read_is_stale() {
    let (server, store) = mk_server().await;
    let tenant = mk_tenant(&store, "ttl").await;
    let sha = shared_artifact(&store, &[&tenant]).await;
    let auth = token(&tenant, &[WORKSPACE_READ]);
    let artifact = call(
        &server,
        &auth,
        "resources/read",
        json!({ "uri": format!("maidan://artifacts/{sha}") }),
    )
    .await;
    let thread = call(
        &server,
        &auth,
        "resources/read",
        json!({ "uri": format!("maidan://threads/{}", tenant.thread.0) }),
    )
    .await;
    assert_eq!(artifact["ttlMs"].as_u64(), Some(caching::RECORD_TTL_MS));
    assert_eq!(thread["ttlMs"].as_u64(), Some(0));
}

#[tokio::test]
async fn server_discover_returns_the_instructions_capabilities_and_versions() {
    let (server, store) = mk_server().await;
    let tenant = mk_tenant(&store, "discover").await;
    let auth = token(&tenant, &[WORKSPACE_READ]);
    let discover = call(&server, &auth, "server/discover", json!({})).await;
    let initialize = call(
        &server,
        &auth,
        "initialize",
        json!({ "protocolVersion": "2025-11-25" }),
    )
    .await;

    assert_eq!(discover["instructions"], INSTRUCTIONS);
    assert_eq!(discover["instructions"], initialize["instructions"]);
    // 2026 `resources.subscribe` means `subscriptions/listen`, which is not
    // implemented. A 2025 handshake still advertises the legacy RPC.
    assert!(discover["capabilities"]["resources"]
        .get("subscribe")
        .is_none());
    assert_eq!(initialize["capabilities"]["resources"]["subscribe"], true);
    assert_eq!(
        discover["capabilities"]["tools"],
        initialize["capabilities"]["tools"]
    );
    assert_eq!(
        discover["capabilities"]["prompts"],
        initialize["capabilities"]["prompts"]
    );
    let negotiated_2026 = call(
        &server,
        &auth,
        "initialize",
        json!({ "protocolVersion": "2026-07-28" }),
    )
    .await;
    assert_eq!(negotiated_2026["protocolVersion"], "2026-07-28");
    assert_eq!(negotiated_2026["capabilities"], discover["capabilities"]);
    assert_eq!(
        discover["supportedVersions"],
        json!(SUPPORTED_PROTOCOL_VERSIONS)
    );
    assert_eq!(
        discover["_meta"]["io.modelcontextprotocol/serverInfo"],
        initialize["serverInfo"]
    );
    assert_eq!(discover["resultType"], "complete");
}

/// The handshake the official TypeScript SDK and the Inspector run. The new
/// fields are additive: `initialize` itself is not a cacheable result, and it
/// still echoes the client's revision.
#[tokio::test]
async fn a_2025_client_still_initializes() {
    let (server, store) = mk_server().await;
    let tenant = mk_tenant(&store, "legacy").await;
    let auth = token(&tenant, &[WORKSPACE_READ]);
    for revision in ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"] {
        let init = call(
            &server,
            &auth,
            "initialize",
            json!({
                "protocolVersion": revision,
                "capabilities": {},
                "clientInfo": { "name": "sdk", "version": "2.0" }
            }),
        )
        .await;
        assert_eq!(init["protocolVersion"], revision);
        assert_eq!(init["instructions"], INSTRUCTIONS);
        assert_eq!(init["capabilities"]["resources"]["subscribe"], true);
        assert_eq!(init["serverInfo"]["name"], "maidan");
        assert!(
            init.get("ttlMs").is_none(),
            "{revision}: initialize is not cacheable"
        );
        let tools = call(&server, &auth, "tools/list", json!({})).await;
        assert!(tools["tools"].as_array().is_some_and(|t| !t.is_empty()));
    }
}

#[tokio::test]
async fn every_result_says_it_is_complete() {
    let (server, store) = mk_server().await;
    let tenant = mk_tenant(&store, "complete").await;
    let auth = token(&tenant, &[WORKSPACE_READ]);
    for (method, params) in [
        ("initialize", json!({})),
        ("tools/list", json!({})),
        ("tools/call", json!({ "name": "whoami", "arguments": {} })),
        ("prompts/list", json!({})),
    ] {
        let result = call(&server, &auth, method, params).await;
        assert_eq!(result["resultType"], "complete", "{method}");
    }
}

/// `"public"` lets a shared gateway hand one caller's result to another, so a
/// result marked public must be the same bytes for callers in two workspaces
/// holding different capabilities. The capability-filtered catalog is not, and
/// is marked private.
#[tokio::test]
async fn a_public_result_is_the_same_bytes_for_two_tenants_and_a_private_one_need_not_be() {
    let (server, store) = mk_server().await;
    let reader = token(&mk_tenant(&store, "tenant-a").await, &[WORKSPACE_READ]);
    let worker = token(
        &mk_tenant(&store, "tenant-b").await,
        &[WORKSPACE_READ, THREAD_TRANSITION],
    );

    for method in [
        "server/discover",
        "tools/list",
        "prompts/list",
        "resources/list",
        "resources/templates/list",
    ] {
        let a = call(&server, &reader, method, json!({})).await;
        let b = call(&server, &worker, method, json!({})).await;
        assert_eq!(a["cacheScope"], b["cacheScope"], "{method}");
        if a["cacheScope"] == CacheScope::Public.as_str() {
            assert_eq!(
                serde_json::to_vec(&a).unwrap(),
                serde_json::to_vec(&b).unwrap(),
                "{method} is public but differs between tenants"
            );
        }
    }
    let a = call(&server, &reader, "tools/list", json!({})).await;
    let b = call(&server, &worker, "tools/list", json!({})).await;
    assert_ne!(
        a["tools"], b["tools"],
        "the full catalog is filtered per token"
    );
    assert_eq!(a["cacheScope"], "private");
}

/// The cache-hint table in docs/Protocols.md is what integrators plan
/// against, so each row's TTL and scope must be the server's.
#[test]
fn the_documented_cache_hints_are_the_servers() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/Protocols.md");
    let doc = std::fs::read_to_string(path).unwrap();
    let artifact = format!("maidan://artifacts/{}", "c".repeat(64));
    let rows = [
        ("`server/discover`", caching::DISCOVER),
        (
            "`tools/list` on `/mcp`, `/mcp/streamable`",
            caching::TOOLS_LIST,
        ),
        ("`prompts/list`", caching::PROMPTS_LIST),
        (
            "`resources/templates/list`",
            caching::RESOURCE_TEMPLATES_LIST,
        ),
        ("`resources/list`", caching::RESOURCES_LIST),
        (
            "`resources/read` of `maidan://artifacts/{sha256}`",
            caching::resource_read(&artifact),
        ),
        (
            "`resources/read` of `maidan://workspaces/{id}`, `maidan://channels/{id}`",
            caching::resource_read("maidan://channels/x"),
        ),
        (
            "`resources/read` of `maidan://threads/{id}`",
            caching::resource_read("maidan://threads/x"),
        ),
    ];
    for (result, hint) in rows {
        let row = format!("| {result} | {} | `{}` |", hint.ttl_ms, hint.scope.as_str());
        assert!(
            doc.lines().any(|line| line.starts_with(&row)),
            "docs/Protocols.md has no cache-hint row starting {row:?}"
        );
    }
}
