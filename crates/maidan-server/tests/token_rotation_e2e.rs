//! Rotating a token replaces its secret and nothing else.
//!
//! A holder rotates the token it is using without `token:admin`; anyone else
//! needs `token:admin` in the token's workspace. The successor keeps the
//! member, capabilities and quotas, the tokens derived from the old one keep
//! working under it, and the old secret stops working in the same transaction.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApiTokenId, MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, TokenQuota, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    server: tokio::task::JoinHandle<()>,
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
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
    Harness {
        addr,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        store,
        server,
        _dir: dir,
    }
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id
}

async fn token(
    store: &dyn Store,
    ws: WorkspaceId,
    member_id: MemberId,
    capabilities: &[&str],
) -> (ApiTokenId, String) {
    let secret = TokenSecret::generate();
    let record = store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("agent".into()),
            capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    (record.id, secret.as_str().to_string())
}

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

impl Harness {
    async fn rotate(&self, bearer: &str, id: ApiTokenId) -> (StatusCode, Value) {
        let resp = self
            .client
            .post(format!("http://{}/tokens/{}/rotate", self.addr, id.0))
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn reads(&self, bearer: &str, ws: WorkspaceId) -> StatusCode {
        self.client
            .get(format!("http://{}/workspaces/{}/channels", self.addr, ws.0))
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap()
            .status()
    }
}

#[tokio::test]
async fn rotation_replaces_the_secret_and_keeps_the_authority() {
    let h = spawn().await;
    let alpha = workspace(h.store.as_ref(), "alpha").await;
    let bravo = workspace(h.store.as_ref(), "bravo").await;
    let agent = member(h.store.as_ref(), alpha, "agent").await;
    let admin = member(h.store.as_ref(), alpha, "admin").await;
    let outsider = member(h.store.as_ref(), bravo, "outsider").await;
    let (agent_id, agent_secret) = token(
        h.store.as_ref(),
        alpha,
        agent,
        &[capability::WORKSPACE_READ, capability::MESSAGE_POST],
    )
    .await;
    let (admin_id, admin_secret) =
        token(h.store.as_ref(), alpha, admin, &[capability::TOKEN_ADMIN]).await;
    let (_, outsider_secret) = token(
        h.store.as_ref(),
        bravo,
        outsider,
        &[capability::TOKEN_ADMIN],
    )
    .await;
    let quota = TokenQuota {
        capability: capability::MESSAGE_POST.into(),
        max_per_window: 5,
        window_secs: 60,
    };
    h.store
        .replace_token_quotas(agent_id, std::slice::from_ref(&quota))
        .await
        .unwrap();

    // A token derived from the agent's, before the rotation.
    let child: Value = h
        .client
        .post(format!("http://{}/tokens/attenuate", h.addr))
        .bearer_auth(&agent_secret)
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let child_secret = child["secret"].as_str().unwrap().to_string();

    // Someone else's token needs token:admin, and the agent has none.
    let (status, _) = h.rotate(&agent_secret, admin_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // token:admin in another workspace is not token:admin here.
    let (status, _) = h.rotate(&outsider_secret, agent_id).await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "{status}"
    );

    // The holder rotates its own token without token:admin.
    let (status, rotated) = h.rotate(&agent_secret, agent_id).await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    let successor = ApiTokenId(rotated["id"].as_str().unwrap().parse().unwrap());
    let new_secret = rotated["secret"].as_str().unwrap().to_string();
    assert_ne!(successor, agent_id);
    assert_eq!(rotated["member_id"], json!(agent.0));
    assert_eq!(
        rotated["capabilities"],
        json!([capability::WORKSPACE_READ, capability::MESSAGE_POST])
    );
    assert_eq!(
        rotated["quotas"],
        json!([{
            "capability": capability::MESSAGE_POST,
            "max_per_window": 5,
            "window_secs": 60
        }]),
        "the response carries the quotas read before the secret changed"
    );
    assert_eq!(
        h.store.list_token_quotas(successor).await.unwrap(),
        vec![quota],
        "a rotation cannot shed a quota"
    );

    assert_eq!(
        h.reads(&agent_secret, alpha).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(h.reads(&new_secret, alpha).await, StatusCode::OK);
    assert_eq!(
        h.reads(&child_secret, alpha).await,
        StatusCode::OK,
        "a derived token keeps working under the successor"
    );
    let (status, _) = h.rotate(&agent_secret, agent_id).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the old secret is gone");

    // An admin rotates the agent's new token; the derived one still follows.
    let (status, again) = h.rotate(&admin_secret, successor).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    let third = ApiTokenId(again["id"].as_str().unwrap().parse().unwrap());
    assert_eq!(h.reads(&new_secret, alpha).await, StatusCode::UNAUTHORIZED);
    assert_eq!(h.reads(&child_secret, alpha).await, StatusCode::OK);
    h.store.revoke_api_token(third).await.unwrap();
    assert_eq!(
        h.reads(&child_secret, alpha).await,
        StatusCode::UNAUTHORIZED,
        "revoking the successor still revokes what was derived"
    );

    let audit = h.store.list_audit_for_workspace(alpha, 50).await.unwrap();
    let rotations: Vec<_> = audit
        .iter()
        .filter(|a| a.action == "token.rotate")
        .collect();
    assert_eq!(rotations.len(), 2, "each rotation is audited");
    assert!(rotations
        .iter()
        .any(|a| a.metadata["replaces"] == json!(agent_id.0)));
    h.server.abort();
}

#[tokio::test]
async fn an_agent_rotates_its_own_token_over_mcp() {
    let h = spawn().await;
    let alpha = workspace(h.store.as_ref(), "alpha").await;
    let agent = member(h.store.as_ref(), alpha, "agent").await;
    let (old_id, old_secret) = token(
        h.store.as_ref(),
        alpha,
        agent,
        &[capability::WORKSPACE_READ],
    )
    .await;
    let resp: Value = h
        .client
        .post(format!("http://{}/mcp", h.addr))
        .bearer_auth(&old_secret)
        .header("MCP-Protocol-Version", "2026-07-28")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "rotate_token", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("a tool result: {resp}"));
    let rotated: Value = serde_json::from_str(text).unwrap();
    assert_ne!(rotated["id"], json!(old_id.0));
    let new_secret = rotated["secret"].as_str().unwrap();
    assert_eq!(h.reads(&old_secret, alpha).await, StatusCode::UNAUTHORIZED);
    assert_eq!(h.reads(new_secret, alpha).await, StatusCode::OK);
    h.server.abort();
}
