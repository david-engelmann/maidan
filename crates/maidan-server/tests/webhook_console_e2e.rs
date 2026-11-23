//! The console's twins of the outbound webhook routes. A person signed in
//! sees, creates and revokes a workspace's webhooks only when they are a human
//! admin of it: holding `workspace:admin` on a live token of their own. Another
//! member, an agent, a session made from a token and a person signed in to
//! another workspace are refused, and the twins keep the bearer routes' bar for
//! a token.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router,
    state::WebhookRuntime,
    AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MaidanSession, MemberId, MemberKind, NewApiToken, NewMaidanSession, NewMember, NewWorkspace,
    WorkspaceId,
};
use reqwest::{header, redirect::Policy, StatusCode};
use serde_json::{json, Value};

const SECRET: &[u8] = b"webhook-console-e2e-secret-32-bytes!!!!";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
}

async fn spawn(with_key: bool) -> Env {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    std::mem::forget(dir);
    if with_key {
        state.webhooks = WebhookRuntime::new(Some(Arc::new([7u8; 32])));
    }
    // First-admin minting is off, so a member who holds no admin token is not
    // let in as the workspace's first admin.
    state.oidc = Some(Arc::new(OidcRuntime {
        settings: OidcSettings {
            enabled: true,
            mock: true,
            issuer: "https://mock.idp.local".into(),
            redirect_uri: "http://127.0.0.1/auth/oidc/callback".into(),
            auto_provision: false,
            link_email: false,
            session_ttl_secs: 3600,
            pending_ttl_secs: 600,
            cookie_secure: false,
            post_logout_redirect_uri: None,
            first_admin_mint: false,
            auto_mint: false,
            signup: false,
            signup_email_domains: vec![],
            signup_open: false,
            emails_verified: false,
            privacy_url: None,
            terms_url: None,
        },
        session_secret: Arc::from(SECRET),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Env {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
    }
}

struct Person {
    workspace: WorkspaceId,
    member: MemberId,
    cookie: String,
}

impl Env {
    async fn workspace(&self, name: &str) -> WorkspaceId {
        self.store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap()
            .id
    }

    /// A member of `workspace` holding a live token with `capabilities`, and
    /// the bearer secret.
    async fn member_with_token(
        &self,
        workspace: WorkspaceId,
        handle: &str,
        kind: MemberKind,
        capabilities: &[&str],
    ) -> (MemberId, String) {
        let member = self
            .store
            .create_member(NewMember {
                workspace_id: workspace,
                handle: handle.into(),
                display_name: None,
                kind,
            })
            .await
            .unwrap();
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: workspace,
                member_id: member.id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
                expires_at: None,
            })
            .await
            .unwrap();
        (member.id, secret.as_str().to_string())
    }

    /// A person signed in as `member`: the cookie of a browser session.
    async fn signed_in(&self, workspace: WorkspaceId, member: MemberId) -> Person {
        let session: MaidanSession = self
            .store
            .create_session(NewMaidanSession {
                workspace_id: workspace,
                member_id: member,
                api_token_id: None,
                oidc_identity_id: None,
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            })
            .await
            .unwrap();
        let mut headers = axum::http::HeaderMap::new();
        maidan_server::session::set_session_cookie(&mut headers, session.id, 3600, false, SECRET)
            .unwrap();
        let cookie = headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        Person {
            workspace,
            member,
            cookie,
        }
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        cookie: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header(header::COOKIE, cookie)
            .header(header::ORIGIN, &self.base);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn bearer(
        &self,
        method: reqwest::Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> StatusCode {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        req.send().await.unwrap().status()
    }
}

fn new_hook() -> Value {
    json!({ "url": "https://example.com/hook", "event_kinds": ["message_posted"] })
}

#[tokio::test]
async fn an_admins_session_lists_creates_and_revokes_a_webhook() {
    let env = spawn(true).await;
    let ws = env.workspace("acme").await;
    let (admin, _) = env
        .member_with_token(
            ws,
            "admin",
            MemberKind::Human,
            &["workspace:admin", "workspace:read", "workspace:write"],
        )
        .await;
    let ada = env.signed_in(ws, admin).await;
    let base = format!("/ui/api/workspaces/{}/webhooks", ws.0);

    let (status, listed) = env
        .call(reqwest::Method::GET, &base, &ada.cookie, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed, json!([]));

    let (status, minted) = env
        .call(reqwest::Method::POST, &base, &ada.cookie, Some(new_hook()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{minted}");
    assert!(minted["secret"].as_str().is_some_and(|s| !s.is_empty()));
    let id = minted["webhook"]["id"].as_str().unwrap().to_string();

    let (_, listed) = env
        .call(reqwest::Method::GET, &base, &ada.cookie, None)
        .await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert!(
        !listed
            .to_string()
            .contains(minted["secret"].as_str().unwrap()),
        "the list carries the signing secret"
    );

    let (status, _) = env
        .call(
            reqwest::Method::DELETE,
            &format!("{base}/{id}"),
            &ada.cookie,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, listed) = env
        .call(reqwest::Method::GET, &base, &ada.cookie, None)
        .await;
    assert_eq!(listed, json!([]));
}

#[tokio::test]
async fn a_session_that_is_not_a_workspace_admins_is_refused() {
    let env = spawn(true).await;
    let ws = env.workspace("acme").await;
    let (admin, _) = env
        .member_with_token(
            ws,
            "admin",
            MemberKind::Human,
            &["workspace:admin", "workspace:read", "workspace:write"],
        )
        .await;
    let (reader, _) = env
        .member_with_token(ws, "reader", MemberKind::Human, &["workspace:read"])
        .await;
    let (agent, _) = env
        .member_with_token(
            ws,
            "agent",
            MemberKind::Agent,
            &["workspace:admin", "workspace:read", "workspace:write"],
        )
        .await;
    let admin = env.signed_in(ws, admin).await;
    let base = format!("/ui/api/workspaces/{}/webhooks", ws.0);
    let (_, minted) = env
        .call(
            reqwest::Method::POST,
            &base,
            &admin.cookie,
            Some(new_hook()),
        )
        .await;
    let id = minted["webhook"]["id"].as_str().unwrap().to_string();

    for who in [reader, agent] {
        let person = env.signed_in(ws, who).await;
        for (method, path, body) in [
            (reqwest::Method::GET, base.clone(), None),
            (reqwest::Method::POST, base.clone(), Some(new_hook())),
            (reqwest::Method::DELETE, format!("{base}/{id}"), None),
        ] {
            let (status, _) = env.call(method.clone(), &path, &person.cookie, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        }
    }
    let (_, listed) = env
        .call(reqwest::Method::GET, &base, &admin.cookie, None)
        .await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "nothing was revoked");
}

#[tokio::test]
async fn an_admin_of_another_workspace_is_refused() {
    let env = spawn(true).await;
    let ours = env.workspace("ours").await;
    let theirs = env.workspace("theirs").await;
    let (admin, _) = env
        .member_with_token(
            theirs,
            "admin",
            MemberKind::Human,
            &["workspace:admin", "workspace:read", "workspace:write"],
        )
        .await;
    let outsider = env.signed_in(theirs, admin).await;
    let base = format!("/ui/api/workspaces/{}/webhooks", ours.0);
    for (method, body) in [
        (reqwest::Method::GET, None),
        (reqwest::Method::POST, Some(new_hook())),
    ] {
        let (status, _) = env
            .call(method.clone(), &base, &outsider.cookie, body)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
    }
    assert_eq!(outsider.workspace, theirs);
    assert_eq!(outsider.member, admin);
}

#[tokio::test]
async fn a_token_on_the_twins_is_held_to_the_bearer_routes_bar() {
    let env = spawn(true).await;
    let ws = env.workspace("acme").await;
    let (_, reader) = env
        .member_with_token(ws, "reader", MemberKind::Agent, &["workspace:read"])
        .await;
    let (_, admin) = env
        .member_with_token(
            ws,
            "admin",
            MemberKind::Agent,
            &["workspace:admin", "workspace:read", "workspace:write"],
        )
        .await;
    let base = format!("/ui/api/workspaces/{}/webhooks", ws.0);
    assert_eq!(
        env.bearer(reqwest::Method::GET, &base, &reader, None).await,
        StatusCode::OK
    );
    assert_eq!(
        env.bearer(reqwest::Method::POST, &base, &reader, Some(new_hook()))
            .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.bearer(reqwest::Method::POST, &base, &admin, Some(new_hook()))
            .await,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn creating_a_webhook_without_an_encryption_key_is_a_503_that_names_the_setting() {
    let env = spawn(false).await;
    let ws = env.workspace("acme").await;
    let (admin, token) = env
        .member_with_token(
            ws,
            "admin",
            MemberKind::Human,
            &["workspace:admin", "workspace:read", "workspace:write"],
        )
        .await;
    let ada = env.signed_in(ws, admin).await;
    let (status, problem) = env
        .call(
            reqwest::Method::POST,
            &format!("/ui/api/workspaces/{}/webhooks", ws.0),
            &ada.cookie,
            Some(new_hook()),
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{problem}");
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|d| d.contains("FEDERATION_ENCRYPTION_KEY")),
        "{problem}"
    );
    // The bearer route says the same.
    assert_eq!(
        env.bearer(
            reqwest::Method::POST,
            &format!("/workspaces/{}/webhooks", ws.0),
            &token,
            Some(new_hook())
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}
