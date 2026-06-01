//! Table-driven MCP tool capability denial + allow gate (Cluster 69).

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const FORBIDDEN: i64 = -32003;

fn contract_map() -> BTreeMap<String, String> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/mcp-capability-map.json");
    serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("capability map json")
}

fn deny_caps(required: &str) -> Vec<String> {
    match required {
        capability::WORKSPACE_READ => vec![],
        capability::WORKSPACE_WRITE => vec![capability::WORKSPACE_READ.into()],
        capability::MESSAGE_POST => vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
        capability::ARTIFACT_UPLOAD => vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
        capability::SEARCH_QUERY => vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
        other => panic!("unknown capability in map: {other}"),
    }
}

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Harness {
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    Harness {
        addr,
        server,
        client,
        store,
        _dir: dir,
    }
}

async fn seed_workspace(store: &dyn Store) -> (maidan_types::WorkspaceId, maidan_types::MemberId) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "mcp-cap".to_string(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    (ws.id, member.id)
}

async fn mint_token(
    store: &dyn Store,
    workspace_id: maidan_types::WorkspaceId,
    member_id: maidan_types::MemberId,
    capabilities: Vec<String>,
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn mcp_call(h: &Harness, bearer: &str, tool: &str, args: Value) -> Value {
    h.client
        .post(format!("http://{}/mcp", h.addr))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": args }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn assert_forbidden(resp: &Value, tool: &str) {
    assert_eq!(
        resp["error"]["code"].as_i64(),
        Some(FORBIDDEN),
        "tool {tool}: expected forbidden, got {resp}"
    );
}

fn assert_not_forbidden(resp: &Value, tool: &str) {
    if let Some(code) = resp["error"]["code"].as_i64() {
        assert_ne!(code, FORBIDDEN, "tool {tool}: unexpected forbidden: {resp}");
    }
}

#[tokio::test]
async fn every_mcp_tool_denies_without_required_capability() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let ws = workspace_id.0.to_string();

    for (tool, required) in contract_map() {
        let bearer = mint_token(
            h.store.as_ref(),
            workspace_id,
            member_id,
            deny_caps(&required),
        )
        .await;
        let args = if required == capability::WORKSPACE_READ {
            json!({ "workspace_id": ws })
        } else {
            json!({})
        };
        let resp = mcp_call(&h, &bearer, &tool, args).await;
        assert_forbidden(&resp, &tool);
    }

    h.server.abort();
}

#[tokio::test]
async fn every_mcp_tool_passes_capability_gate_with_required_cap() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let ws = workspace_id.0.to_string();

    for (tool, required) in contract_map() {
        let bearer = mint_token(
            h.store.as_ref(),
            workspace_id,
            member_id,
            vec![required.clone()],
        )
        .await;
        let args = if required == capability::WORKSPACE_READ {
            json!({ "workspace_id": ws })
        } else {
            json!({})
        };
        let resp = mcp_call(&h, &bearer, &tool, args).await;
        assert_not_forbidden(&resp, &tool);
    }

    h.server.abort();
}
