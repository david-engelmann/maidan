//! `POST /workspaces/{wid}/agents`: a `token:admin` holder connects an agent
//! (member plus `maidan.agent.worker` token) on a server with bootstrap off,
//! where `POST /workspaces/{wid}/members` is refused. Refusals create nothing,
//! and another tenant's admin cannot create an agent here.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, capability_set, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::new(
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
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    Harness {
        addr,
        server,
        client,
        store,
        _dir: dir,
    }
}

async fn mint(
    store: &dyn Store,
    workspace: maidan_types::WorkspaceId,
    member: maidan_types::MemberId,
    caps: Vec<String>,
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: workspace,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("test".into()),
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

struct Tenant {
    id: maidan_types::WorkspaceId,
    admin: maidan_types::MemberId,
    admin_token: String,
}

async fn tenant(h: &Harness, name: &str) -> Tenant {
    let ws = h
        .store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let admin = h
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "root".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    // A workspace admin: everything but the two cross-tenant capabilities.
    let caps = capability::all()
        .into_iter()
        .filter(|c| c != capability::OPERATOR_GLOBAL && c != capability::AUDIT_READ_GLOBAL)
        .collect();
    let admin_token = mint(h.store.as_ref(), ws.id, admin.id, caps).await;
    Tenant {
        id: ws.id,
        admin: admin.id,
        admin_token,
    }
}

fn worker_set() -> Vec<String> {
    let mut caps = capability_set::named_sets()
        .into_iter()
        .find(|s| s.name == capability_set::AGENT_WORKER)
        .unwrap()
        .capabilities;
    caps.sort();
    caps
}

async fn handles(h: &Harness, ws: maidan_types::WorkspaceId) -> Vec<String> {
    let mut out: Vec<String> = h
        .store
        .list_members(ws)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.handle)
        .collect();
    out.sort();
    out
}

async fn post_agent(
    h: &Harness,
    ws: maidan_types::WorkspaceId,
    token: Option<&str>,
    body: serde_json::Value,
) -> reqwest::Response {
    let mut req = h
        .client
        .post(format!("{}/workspaces/{}/agents", h.base(), ws.0))
        .json(&body);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap()
}

#[tokio::test]
async fn an_admin_connects_an_agent_with_bootstrap_off() {
    let h = spawn().await;
    let a = tenant(&h, "a").await;

    // The bootstrap route this replaces is refused on this server.
    let bootstrap = h
        .client
        .post(format!("{}/workspaces/{}/members", h.base(), a.id.0))
        .bearer_auth(&a.admin_token)
        .json(&json!({ "handle": "via-bootstrap", "kind": "agent" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bootstrap.status(), StatusCode::FORBIDDEN);

    let res = post_agent(
        &h,
        a.id,
        Some(&a.admin_token),
        json!({ "handle": "builder", "display_name": "Builder" }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let body: serde_json::Value = res.json().await.unwrap();
    let member_id = body["member"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["member"]["kind"], "agent");
    assert_eq!(body["member"]["handle"], "builder");
    assert_eq!(body["member"]["display_name"], "Builder");
    assert_eq!(body["member"]["workspace_id"], a.id.0.to_string());
    assert_eq!(body["token"]["member_id"], member_id.as_str());
    let mut caps: Vec<String> =
        serde_json::from_value(body["token"]["capabilities"].clone()).unwrap();
    caps.sort();
    assert_eq!(caps, worker_set());
    assert!(!caps.iter().any(|c| c == capability::TOKEN_ADMIN));

    // The secret works, as that agent, with exactly the worker set.
    let secret = body["token"]["secret"].as_str().unwrap().to_string();
    let me: serde_json::Value = h
        .client
        .get(format!("{}/me", h.base()))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["member_id"], member_id.as_str());
    assert_eq!(me["workspace_id"], a.id.0.to_string());

    // The agent cannot connect agents of its own.
    let own = post_agent(&h, a.id, Some(&secret), json!({ "handle": "spawned" })).await;
    assert_eq!(own.status(), StatusCode::FORBIDDEN);

    // Both audit rows name the admin who connected it.
    let rows = h.store.list_audit_for_workspace(a.id, 50).await.unwrap();
    let created = rows
        .iter()
        .find(|r| r.action == "member.create")
        .expect("member.create");
    assert_eq!(created.actor_id, Some(a.admin));
    assert_eq!(
        created.target_id.map(|id| id.to_string()),
        Some(member_id.clone())
    );
    let minted = rows
        .iter()
        .find(|r| r.action == "token.mint")
        .expect("token.mint");
    assert_eq!(minted.actor_id, Some(a.admin));
    assert_eq!(
        minted.target_id.map(|id| id.to_string()),
        body["token"]["id"].as_str().map(str::to_string)
    );
    assert_eq!(
        minted.metadata["capability_set"],
        capability_set::AGENT_WORKER
    );

    assert_eq!(
        handles(&h, a.id).await,
        vec!["builder".to_string(), "root".to_string()]
    );
    h.shutdown().await;
}

#[tokio::test]
async fn a_refused_request_creates_nothing() {
    let h = spawn().await;
    let a = tenant(&h, "a").await;
    let narrow = mint(
        h.store.as_ref(),
        a.id,
        a.admin,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
    )
    .await;
    let ok = json!({ "handle": "builder" });

    assert_eq!(
        post_agent(&h, a.id, None, ok.clone()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post_agent(&h, a.id, Some(&narrow), ok.clone())
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    // It never takes a capability list or a kind.
    for body in [
        json!({ "handle": "builder", "capabilities": [capability::TOKEN_ADMIN] }),
        json!({ "handle": "builder", "capability_set": "maidan.human.admin" }),
        json!({ "handle": "builder", "kind": "human" }),
        json!({ "handle": "" }),
        json!({ "handle": "two words" }),
        json!({ "handle": "x".repeat(65) }),
    ] {
        let res = post_agent(&h, a.id, Some(&a.admin_token), body.clone()).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    assert_eq!(handles(&h, a.id).await, vec!["root".to_string()]);

    // A taken handle is a conflict, and leaves no second token behind.
    assert_eq!(
        post_agent(&h, a.id, Some(&a.admin_token), ok.clone())
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_agent(&h, a.id, Some(&a.admin_token), ok.clone())
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let builder = h.store.get_member_by_handle(a.id, "builder").await.unwrap();
    assert_eq!(
        h.store
            .list_api_tokens_for_member(a.id, builder.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let rows = h.store.list_audit_for_workspace(a.id, 50).await.unwrap();
    assert_eq!(
        rows.iter().filter(|r| r.action == "member.create").count(),
        1
    );
    assert_eq!(rows.iter().filter(|r| r.action == "token.mint").count(), 1);
    h.shutdown().await;
}

#[tokio::test]
async fn another_tenants_admin_cannot_create_an_agent_here() {
    let h = spawn().await;
    let a = tenant(&h, "a").await;
    let b = tenant(&h, "b").await;

    let res = post_agent(
        &h,
        a.id,
        Some(&b.admin_token),
        json!({ "handle": "intruder" }),
    )
    .await;
    assert!(
        res.status() == StatusCode::FORBIDDEN || res.status() == StatusCode::NOT_FOUND,
        "{}",
        res.status()
    );
    assert_eq!(handles(&h, a.id).await, vec!["root".to_string()]);
    assert!(h
        .store
        .list_audit_for_workspace(a.id, 50)
        .await
        .unwrap()
        .iter()
        .all(|r| r.action != "member.create" && r.action != "token.mint"));

    // B's own agent lands in B, under the same handle A could still take.
    let own = post_agent(
        &h,
        b.id,
        Some(&b.admin_token),
        json!({ "handle": "intruder" }),
    )
    .await;
    assert_eq!(own.status(), StatusCode::CREATED);
    let body: serde_json::Value = own.json().await.unwrap();
    assert_eq!(body["member"]["workspace_id"], b.id.0.to_string());
    let secret = body["token"]["secret"].as_str().unwrap().to_string();
    assert_eq!(handles(&h, a.id).await, vec!["root".to_string()]);
    // B's agent token reads nothing of A.
    let cross = h
        .client
        .get(format!("{}/workspaces/{}/members", h.base(), a.id.0))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert!(cross.status() == StatusCode::FORBIDDEN || cross.status() == StatusCode::NOT_FOUND);
    assert_eq!(
        post_agent(
            &h,
            a.id,
            Some(&a.admin_token),
            json!({ "handle": "intruder" })
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    h.shutdown().await;
}
