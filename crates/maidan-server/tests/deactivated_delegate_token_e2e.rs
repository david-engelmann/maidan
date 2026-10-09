//! A delegated token stops working when its **delegate** is deactivated or
//! deprovisioned through SCIM (found in #1253).
//!
//! Deactivating a member revokes the tokens that member holds
//! (`maidan_api_tokens.member_id = member`). A delegated token belongs to the
//! grant's *subject*, so it isn't one of them, and the grant stays live. The
//! bearer check only asks whether the grant is revoked or expired. So the
//! deactivated delegate's exchanged token still passes on REST and MCP.
//!
//! Two tenants: deactivating the delegate in workspace A must refuse A's
//! delegated token, and workspace B's equivalent must keep working.

use std::sync::{atomic::AtomicI64, Arc};

use chrono::{Duration as ChronoDuration, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, MemberId, MemberKind, NewApiToken, NewAuditEvent, NewDelegationGrant, NewMember,
    NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Env {
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
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Env {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        store,
        _dir: dir,
    }
}

/// One tenant: a subject, a SCIM-provisioned delegate holding a grant from
/// the subject, the delegate's own token, and the delegated token it got by
/// exchanging the grant.
struct Tenant {
    ws: WorkspaceId,
    delegate: MemberId,
    delegate_secret: String,
    delegated_secret: String,
}

impl Env {
    async fn member(&self, ws: WorkspaceId, handle: &str, kind: MemberKind) -> MemberId {
        self.store
            .create_member(NewMember {
                workspace_id: ws,
                handle: handle.into(),
                display_name: None,
                kind,
            })
            .await
            .unwrap()
            .id
    }

    async fn tenant(&self, name: &str) -> Tenant {
        let ws = self
            .store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap()
            .id;
        let subject = self.member(ws, "subject", MemberKind::Human).await;
        let delegate = self.member(ws, "delegate", MemberKind::Human).await;
        self.store
            .create_scim_user(delegate, ws, None, true)
            .await
            .unwrap();
        let grant = self
            .store
            .create_delegation_grant(NewDelegationGrant {
                workspace_id: ws,
                subject_id: subject,
                delegate_id: delegate,
                capabilities: vec![capability::WORKSPACE_READ.into()],
                purpose: "cover while away".into(),
                authorized_by: subject,
                expires_at: Utc::now() + ChronoDuration::hours(2),
            })
            .await
            .unwrap();
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: ws,
                member_id: delegate,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: vec![capability::WORKSPACE_READ.into()],
                expires_at: None,
            })
            .await
            .unwrap();
        let delegate_secret = secret.as_str().to_string();
        let resp = self
            .client
            .post(format!("{}/tokens/delegate", self.base))
            .bearer_auth(&delegate_secret)
            .json(&json!({ "grant_id": grant.id.0 }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "grant exchange");
        let exchanged: Value = resp.json().await.unwrap();
        assert_eq!(exchanged["token"]["member_id"], subject.0.to_string());
        let delegated_secret = exchanged["token"]["secret"].as_str().unwrap().to_string();
        Tenant {
            ws,
            delegate,
            delegate_secret,
            delegated_secret,
        }
    }

    /// An ordinary REST read of the tenant's workspace.
    async fn rest(&self, t: &Tenant, secret: &str) -> StatusCode {
        self.client
            .get(format!("{}/workspaces/{}", self.base, t.ws.0))
            .bearer_auth(secret)
            .send()
            .await
            .unwrap()
            .status()
    }

    /// An MCP `tools/call` of the read-only `whoami` tool. A refused bearer is
    /// a 401/403. An accepted one is a 200 whose body holds a result or an
    /// error, so a JSON-RPC error counts as refused too.
    async fn mcp_call(&self, secret: &str) -> (StatusCode, Value) {
        let resp = self
            .client
            .post(format!("{}/mcp", self.base))
            .bearer_auth(secret)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "whoami", "arguments": {} }
            }))
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let body = resp.json().await.unwrap_or(Value::Null);
        (status, body)
    }

    async fn deactivate(&self, ws: WorkspaceId, member: MemberId) {
        self.store
            .scim_update_user_audited(
                ws,
                member,
                None,
                None,
                false,
                NewAuditEvent {
                    scope: AuditScope::Workspace(ws),
                    actor_id: None,
                    action: "scim.user.update".into(),
                    target_kind: Some("member".into()),
                    target_id: Some(member.0),
                    metadata: json!({ "active": false }),
                },
            )
            .await
            .unwrap()
            .expect("SCIM user");
    }
}

fn refused(status: StatusCode) -> bool {
    status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN
}

fn mcp_refused((status, body): &(StatusCode, Value)) -> bool {
    refused(*status) || body.get("error").is_some()
}

fn mcp_accepted((status, body): &(StatusCode, Value)) -> bool {
    *status == StatusCode::OK && body.get("result").is_some() && body.get("error").is_none()
}

#[tokio::test]
async fn a_deactivated_delegates_token_is_refused_only_in_its_workspace() {
    let env = spawn().await;
    let a = env.tenant("Alpha").await;
    let b = env.tenant("Bravo").await;

    // Before: both delegated tokens work on REST and MCP.
    for t in [&a, &b] {
        assert_eq!(env.rest(t, &t.delegated_secret).await, StatusCode::OK);
        let mcp = env.mcp_call(&t.delegated_secret).await;
        assert!(mcp_accepted(&mcp), "before deactivation: {mcp:?}");
    }

    env.deactivate(a.ws, a.delegate).await;

    // Deactivation took effect for the delegate's own token.
    assert_eq!(
        env.rest(&a, &a.delegate_secret).await,
        StatusCode::UNAUTHORIZED,
        "precondition: the deactivated delegate's own token is revoked"
    );

    // Workspace B is untouched.
    assert_eq!(env.rest(&b, &b.delegated_secret).await, StatusCode::OK);
    let b_mcp = env.mcp_call(&b.delegated_secret).await;
    assert!(
        mcp_accepted(&b_mcp),
        "workspace B's delegated token: {b_mcp:?}"
    );

    // The finding: A's delegated token, exchanged by the now-deactivated
    // delegate, must be refused on REST and on MCP.
    let a_rest = env.rest(&a, &a.delegated_secret).await;
    let a_mcp = env.mcp_call(&a.delegated_secret).await;
    assert!(
        refused(a_rest) && mcp_refused(&a_mcp),
        "a deactivated delegate's delegated token was accepted: REST GET /workspaces/{{A}} -> {a_rest}, MCP tools/call whoami -> {} {}",
        a_mcp.0,
        a_mcp.1
    );
}

/// Deprovisioning deletes the SCIM link rather than marking it inactive, so
/// nothing left behind says the person is gone: the delegate's grants and its
/// signed-in sessions have to end in the same transaction.
#[tokio::test]
async fn a_deprovisioned_delegate_loses_its_delegated_token_and_its_sessions() {
    let env = spawn().await;
    let a = env.tenant("Alpha").await;
    let b = env.tenant("Bravo").await;
    let session = env
        .store
        .create_session(maidan_types::NewMaidanSession {
            workspace_id: a.ws,
            member_id: a.delegate,
            api_token_id: None,
            expires_at: Utc::now() + ChronoDuration::hours(1),
        })
        .await
        .unwrap();

    assert!(env
        .store
        .scim_deprovision_audited(
            a.ws,
            a.delegate,
            NewAuditEvent {
                scope: AuditScope::Workspace(a.ws),
                actor_id: None,
                action: "scim.user.delete".into(),
                target_kind: Some("member".into()),
                target_id: Some(a.delegate.0),
                metadata: json!({}),
            },
        )
        .await
        .unwrap());

    assert!(refused(env.rest(&a, &a.delegated_secret).await));
    assert!(mcp_refused(&env.mcp_call(&a.delegated_secret).await));
    assert!(
        env.store.get_session(session.id).await.is_err(),
        "the deprovisioned delegate's session must be gone"
    );
    assert_eq!(env.rest(&b, &b.delegated_secret).await, StatusCode::OK);

    let audit = env.store.list_audit(500).await.unwrap();
    assert!(audit.iter().any(|r| r.action == "delegation_grant.revoke"
        && r.metadata["delegate_id"] == json!(a.delegate.0)));
    assert!(audit
        .iter()
        .any(|r| r.action == "session.delete" && r.target_id == Some(a.delegate.0)));
}
