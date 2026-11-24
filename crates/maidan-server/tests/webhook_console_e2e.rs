//! The console's twins of the outbound webhook routes. A person signed in
//! sees, creates and revokes a workspace's webhooks only when they are a human
//! admin of it: holding `workspace:admin` on a live token of their own. Another
//! member, an agent, a session made from a token and a person signed in to
//! another workspace are refused, and the twins keep the bearer routes' bar for
//! a token.

// The receiver is a plain axum server, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc, Mutex},
    time::Duration,
};

use axum::{body::Bytes, extract::State, http::Uri, Router};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{expand_set, hash_secret, TokenSecret, AGENT_WORKER};
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router,
    state::WebhookRuntime,
    webhook_worker::WebhookWorker,
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

type Seen = Arc<Mutex<Vec<(String, Value)>>>;

/// A receiver for every hook: it records each request's path and JSON body.
async fn record(State(seen): State<Seen>, uri: Uri, body: Bytes) -> axum::Json<Value> {
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    seen.lock().unwrap().push((uri.path().to_string(), value));
    axum::Json(json!({ "text": "ok" }))
}

struct Env {
    base: String,
    hooks: String,
    seen: Seen,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _worker: WebhookWorker,
}

async fn spawn(with_key: bool) -> Env {
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    #[allow(unsafe_code)] // edition 2024 makes env writes unsafe; test setup only
    unsafe {
        std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1")
    };
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
    let worker = WebhookWorker::spawn(state.clone());
    let seen: Seen = Arc::default();
    let hooks_app = Router::new().fallback(record).with_state(seen.clone());
    let hooks_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hooks_addr = hooks_listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(hooks_listener, hooks_app).await.unwrap() });
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
        hooks: format!("http://{hooks_addr}"),
        seen,
        _worker: worker,
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
        let capabilities = capabilities.iter().map(|c| c.to_string()).collect();
        self.member_with_caps(workspace, handle, kind, capabilities)
            .await
    }

    async fn member_with_caps(
        &self,
        workspace: WorkspaceId,
        handle: &str,
        kind: MemberKind,
        capabilities: Vec<String>,
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
                capabilities,
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

impl Env {
    /// Wait up to ten seconds for `/{name}` to get a body of kind `kind`.
    async fn delivered(&self, name: &str, kind: &str) -> bool {
        let path = format!("/{name}");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            if self
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|(p, body)| *p == path && body["kind"] == kind)
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }
}

impl Env {
    /// Another live token for `member`, optionally expiring.
    async fn extra_token(
        &self,
        workspace: WorkspaceId,
        member: MemberId,
        capabilities: &[&str],
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> maidan_types::ApiToken {
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: workspace,
                member_id: member,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
                expires_at,
            })
            .await
            .unwrap()
    }

    async fn general_thread(&self, workspace: WorkspaceId) -> maidan_types::ThreadId {
        let channel = self
            .store
            .create_channel(maidan_types::NewChannel {
                workspace_id: workspace,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap()
            .id;
        self.store
            .create_thread(maidan_types::NewThread {
                channel_id: channel,
                parent_thread_id: None,
                title: Some("work".into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap()
            .id
    }

    async fn say(&self, thread: maidan_types::ThreadId, poster: &str, text: &str) {
        let res = self
            .client
            .post(format!("{}/threads/{}/messages", self.base, thread.0))
            .bearer_auth(poster)
            .json(&json!({ "body": text }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }
}

const ADMIN_CAPS: &[&str] = &["workspace:admin", "workspace:read", "workspace:write"];

#[tokio::test]
async fn a_webhook_made_in_the_console_acts_as_the_members_own_token_and_delivers() {
    let env = spawn(true).await;
    let ws = env.workspace("acme").await;
    let (admin, _) = env
        .member_with_token(ws, "admin", MemberKind::Human, ADMIN_CAPS)
        .await;
    let (_, poster) = env
        .member_with_caps(
            ws,
            "poster",
            MemberKind::Agent,
            expand_set(AGENT_WORKER).unwrap(),
        )
        .await;
    let thread = env.general_thread(ws).await;
    let own = env
        .store
        .list_api_tokens_for_member(ws, admin)
        .await
        .unwrap()
        .remove(0);
    // Two more qualifying tokens, both expiring: the one that never expires
    // is still the one the webhook acts as.
    env.extra_token(
        ws,
        admin,
        ADMIN_CAPS,
        Some(chrono::Utc::now() + chrono::Duration::days(30)),
    )
    .await;
    env.extra_token(
        ws,
        admin,
        ADMIN_CAPS,
        Some(chrono::Utc::now() + chrono::Duration::days(90)),
    )
    .await;
    let tokens_before = env
        .store
        .list_api_tokens_for_member(ws, admin)
        .await
        .unwrap()
        .len();
    let person = env.signed_in(ws, admin).await;
    let base = format!("/ui/api/workspaces/{}/webhooks", ws.0);

    let (status, minted) = env
        .call(
            reqwest::Method::POST,
            &base,
            &person.cookie,
            Some(json!({ "url": format!("{}/console", env.hooks), "event_kinds": ["message_posted"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{minted}");
    let id = maidan_types::WebhookSubscriptionId(
        minted["webhook"]["id"].as_str().unwrap().parse().unwrap(),
    );

    // It records the member's own token, and mints nothing.
    let held = env.store.get_webhook_subscription(id).await.unwrap();
    assert_eq!(held.created_by_token, Some(own.id));
    assert_eq!(
        env.store
            .list_api_tokens_for_member(ws, admin)
            .await
            .unwrap()
            .len(),
        tokens_before,
        "creating a webhook made a token"
    );

    // It delivers, and is still there afterwards.
    env.say(thread, &poster, "hello").await;
    assert!(
        env.delivered("console", "message_posted").await,
        "a webhook made in the console never delivered"
    );
    let (_, listed) = env
        .call(reqwest::Method::GET, &base, &person.cookie, None)
        .await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
}

#[tokio::test]
async fn revoking_the_token_a_console_webhook_acts_as_ends_it_and_leaves_the_person_no_extra_ability(
) {
    let env = spawn(true).await;
    let ws = env.workspace("acme").await;
    let (admin, _) = env
        .member_with_token(ws, "admin", MemberKind::Human, ADMIN_CAPS)
        .await;
    let (_, poster) = env
        .member_with_caps(
            ws,
            "poster",
            MemberKind::Agent,
            expand_set(AGENT_WORKER).unwrap(),
        )
        .await;
    let thread = env.general_thread(ws).await;
    let person = env.signed_in(ws, admin).await;
    let base = format!("/ui/api/workspaces/{}/webhooks", ws.0);
    let own = env
        .store
        .list_api_tokens_for_member(ws, admin)
        .await
        .unwrap()
        .remove(0);
    let (status, minted) = env
        .call(
            reqwest::Method::POST,
            &base,
            &person.cookie,
            Some(
                json!({ "url": format!("{}/ends", env.hooks), "event_kinds": ["message_posted"] }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{minted}");
    let id = maidan_types::WebhookSubscriptionId(
        minted["webhook"]["id"].as_str().unwrap().parse().unwrap(),
    );
    env.say(thread, &poster, "first").await;
    assert!(env.delivered("ends", "message_posted").await);

    // The token goes. The person's admin standing goes with it, since nothing
    // else holds it for them: no token was made for the webhook.
    env.store.revoke_api_token(own.id).await.unwrap();
    assert_eq!(
        env.store
            .list_api_tokens_for_member(ws, admin)
            .await
            .unwrap()
            .len(),
        1,
        "a token was made for the webhook"
    );
    let (status, _) = env
        .call(reqwest::Method::GET, &base, &person.cookie, None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The next matching event finds its authority gone and ends the webhook.
    env.say(thread, &poster, "second").await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let held = env.store.get_webhook_subscription(id).await.unwrap();
        if held.subscription.revoked_at.is_some() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the webhook outlived its token"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn a_session_whose_member_holds_no_token_with_both_capabilities_gets_a_403_and_nothing_is_stored(
) {
    let env = spawn(true).await;
    let ws = env.workspace("acme").await;
    // Admin on one token and read on another, but never both on one.
    let (admin, _) = env
        .member_with_token(ws, "admin", MemberKind::Human, &["workspace:admin"])
        .await;
    env.extra_token(ws, admin, &["workspace:read"], None).await;
    let person = env.signed_in(ws, admin).await;
    let base = format!("/ui/api/workspaces/{}/webhooks", ws.0);
    let tokens_before = env
        .store
        .list_api_tokens_for_member(ws, admin)
        .await
        .unwrap()
        .len();

    let (status, problem) = env
        .call(
            reqwest::Method::POST,
            &base,
            &person.cookie,
            Some(new_hook()),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(
        problem["detail"],
        "a webhook acts as one of your tokens, and you hold none with workspace:admin and workspace:read"
    );
    assert!(env
        .store
        .list_webhook_subscriptions(ws)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        env.store
            .list_api_tokens_for_member(ws, admin)
            .await
            .unwrap()
            .len(),
        tokens_before,
        "a refused create made a token"
    );
}
