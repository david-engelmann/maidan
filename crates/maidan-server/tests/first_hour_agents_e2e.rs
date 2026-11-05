//! The first hour on a hosted workspace: a sign-up opens with one starter
//! task in #general, and the person who signed up adds an agent from the
//! console (`POST /ui/api/workspaces/{wid}/agents`) without ever holding an
//! admin token.
//!
//! Two tenants throughout: each sign-up gets its own workspace, and a session
//! for one cannot add an agent to the other. A session made from a token, a
//! request from another origin and a member who is not an admin are refused
//! and create nothing; the agent's token is the worker set, never an admin's.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, capability_set, hash_secret, TokenSecret};
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewMaidanSession, NewMember, ThreadState, WorkspaceId,
};
use reqwest::{redirect::Policy, StatusCode};
use serde_json::{json, Value};

const SECRET: &[u8] = b"first-hour-agents-e2e-secret-32-bytes!!";
const ISSUER: &str = "https://mock.idp.local";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

/// A server with open sign-up. `rate_limits` turns the built-in limits on,
/// as the server's own boot does.
async fn spawn(rate_limits: bool) -> Env {
    let pool = maidan_search::sqlite_pool_options()
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
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
    state.rate_limit_default_on = rate_limits;
    state.oidc = Some(Arc::new(OidcRuntime {
        settings: OidcSettings {
            enabled: true,
            mock: true,
            issuer: ISSUER.into(),
            redirect_uri: "http://127.0.0.1/auth/oidc/callback".into(),
            auto_provision: false,
            link_email: false,
            session_ttl_secs: 3600,
            pending_ttl_secs: 600,
            cookie_secure: false,
            post_logout_redirect_uri: None,
            first_admin_mint: true,
            auto_mint: false,
            signup: true,
            signup_email_domains: vec![],
            signup_open: true,
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
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        store,
        _dir: dir,
    }
}

/// A person who signed up: their session cookie, workspace and member.
struct SignedUp {
    cookie: String,
    workspace: WorkspaceId,
    member: MemberId,
}

impl Env {
    /// Sign up through the mock provider and return the new session.
    async fn sign_up(&self, subject: &str, email: &str) -> SignedUp {
        let start = self
            .client
            .get(format!("{}/auth/oidc/signup", self.base))
            .send()
            .await
            .unwrap();
        assert_eq!(
            start.status(),
            StatusCode::TEMPORARY_REDIRECT,
            "sign-up start"
        );
        let location = start.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap()
            .replace("mock_sub=mock-user", &format!("mock_sub={subject}"))
            .replace(
                "mock_email=human@example.com",
                &format!("mock_email={email}"),
            );
        let callback = self
            .client
            .get(format!("{}{location}", self.base))
            .send()
            .await
            .unwrap();
        assert_eq!(
            callback.status(),
            StatusCode::TEMPORARY_REDIRECT,
            "sign-up callback"
        );
        let cookie = callback
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("maidan_session="))
            .map(|v| v.split(';').next().unwrap().to_string())
            .expect("session cookie");
        let rows = self
            .store
            .list_subject_workspaces(ISSUER, subject, 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "one workspace per sign-up");
        SignedUp {
            cookie,
            workspace: rows[0].workspace_id,
            member: rows[0].member_id,
        }
    }

    /// `POST /ui/api/workspaces/{wid}/agents` on a session cookie, as the
    /// console sends it (same origin).
    async fn add_agent(
        &self,
        cookie: &str,
        workspace: WorkspaceId,
        body: Value,
    ) -> (StatusCode, Value) {
        let res = self
            .client
            .post(format!(
                "{}/ui/api/workspaces/{}/agents",
                self.base, workspace.0
            ))
            .header(reqwest::header::COOKIE, cookie)
            .header("sec-fetch-site", "same-origin")
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn agents(&self, workspace: WorkspaceId) -> usize {
        self.store
            .list_members(workspace)
            .await
            .unwrap()
            .iter()
            .filter(|m| m.kind == MemberKind::Agent)
            .count()
    }

    async fn mint(&self, workspace: WorkspaceId, member: MemberId, caps: &[&str]) -> String {
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: workspace,
                member_id: member,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: caps.iter().map(|c| c.to_string()).collect(),
                expires_at: None,
            })
            .await
            .unwrap();
        secret.as_str().to_string()
    }

    /// The cookie of a session someone signed in to, for a member made here.
    async fn signed_in(&self, workspace: WorkspaceId, member: MemberId) -> String {
        let session = self
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
        headers
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }
}

#[tokio::test]
async fn a_fresh_sign_up_opens_with_one_say_hello_task_in_general() {
    let env = spawn(false).await;
    let ada = env.sign_up("sub-ada", "ada@first-hour.example").await;

    let channels = env.store.list_channels(ada.workspace).await.unwrap();
    assert_eq!(channels.len(), 1, "the workspace opens on one channel");
    assert_eq!(channels[0].name, "general");

    let threads = env.store.list_threads(channels[0].id).await.unwrap();
    assert_eq!(threads.len(), 1, "exactly one starter task: {threads:?}");
    let task = &threads[0];
    assert_eq!(task.title.as_deref(), Some("Say hello"));
    assert_eq!(task.state, ThreadState::Open);
    assert_eq!(
        task.created_by,
        Some(ada.member),
        "the new admin owns it, so its review reaches them"
    );

    // Its opening message tells the agent the whole loop.
    let messages = env.store.list_messages(task.id, 10).await.unwrap();
    assert_eq!(messages.len(), 1);
    let body = &messages[0].body;
    for step in ["Claim", "hello", "result", "review"] {
        assert!(body.contains(step), "the message names `{step}`: {body}");
    }
    assert_eq!(messages[0].author_id, ada.member);

    // The person sees it through the console's own read.
    let res = env
        .client
        .get(format!(
            "{}/ui/api/channels/{}/threads",
            env.base, channels[0].id.0
        ))
        .header(reqwest::header::COOKIE, &ada.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let listed: Value = res.json().await.unwrap();
    let open: Vec<&Value> = listed
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["state"] == "open")
        .collect();
    assert_eq!(open.len(), 1, "one open thread: {listed}");
    assert_eq!(open[0]["title"], "Say hello");
}

#[tokio::test]
async fn the_person_who_signed_up_adds_an_agent_with_a_worker_token() {
    let env = spawn(false).await;
    let ada = env.sign_up("sub-ada", "ada@first-hour.example").await;

    let (status, out) = env
        .add_agent(
            &ada.cookie,
            ada.workspace,
            json!({"handle": "claude", "display_name": "Claude"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{out}");
    assert_eq!(out["member"]["handle"], "claude");
    assert_eq!(out["member"]["kind"], "agent");
    assert_eq!(out["member"]["workspace_id"], ada.workspace.0.to_string());
    let secret = out["token"]["secret"].as_str().expect("the secret, once");
    let caps: Vec<String> = serde_json::from_value(out["token"]["capabilities"].clone()).unwrap();
    assert_eq!(
        caps,
        capability_set::expand_set(capability_set::AGENT_WORKER).unwrap()
    );
    assert!(!caps.iter().any(|c| c == capability::TOKEN_ADMIN));

    // The token works, as the agent and in this workspace only.
    let me: Value = env
        .client
        .get(format!("{}/me", env.base))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["member_id"], out["member"]["id"]);
    assert_eq!(me["workspace_id"], ada.workspace.0.to_string());

    // Both rows of the add are audited as the person's, from the console.
    let audit = env
        .store
        .list_audit_for_workspace(ada.workspace, 50)
        .await
        .unwrap();
    for action in ["member.create", "token.mint"] {
        let row = audit
            .iter()
            .find(|e| e.action == action && e.metadata["source"] == "console")
            .unwrap_or_else(|| panic!("an audit row for {action}"));
        assert_eq!(row.actor_id, Some(ada.member));
    }

    // Adding an agent does not make the workspace an admin token, so the
    // person can keep adding them.
    let (status, out) = env
        .add_agent(&ada.cookie, ada.workspace, json!({"handle": "codex"}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{out}");
    assert_eq!(env.agents(ada.workspace).await, 2);
}

#[tokio::test]
async fn a_session_cannot_add_an_agent_to_another_workspace() {
    let env = spawn(false).await;
    let ada = env.sign_up("sub-ada", "ada@first-hour.example").await;
    let bob = env.sign_up("sub-bob", "bob@other-tenant.example").await;
    assert_ne!(ada.workspace, bob.workspace);

    let (status, _) = env
        .add_agent(&ada.cookie, bob.workspace, json!({"handle": "intruder"}))
        .await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "{status}"
    );
    assert_eq!(
        env.agents(bob.workspace).await,
        0,
        "bob's workspace is untouched"
    );
    assert_eq!(
        env.agents(ada.workspace).await,
        0,
        "nor did it land in ada's"
    );

    // Each still adds to their own.
    let (status, _) = env
        .add_agent(&bob.cookie, bob.workspace, json!({"handle": "bobs-agent"}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(env.agents(bob.workspace).await, 1);
    assert_eq!(env.agents(ada.workspace).await, 0);
}

#[tokio::test]
async fn a_session_made_from_a_token_cannot_add_an_agent() {
    let env = spawn(false).await;
    let ada = env.sign_up("sub-ada", "ada@first-hour.example").await;
    // Even a token:admin token's session: that holder has the bearer route.
    let token = env
        .mint(
            ada.workspace,
            ada.member,
            &[capability::WORKSPACE_READ, capability::TOKEN_ADMIN],
        )
        .await;
    let exchanged = env
        .client
        .post(format!("{}/auth/session/from-token", env.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(exchanged.status(), StatusCode::CREATED);
    let cookie = exchanged
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let (status, out) = env
        .add_agent(&cookie, ada.workspace, json!({"handle": "via-token"}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
    assert_eq!(env.agents(ada.workspace).await, 0);

    // The token itself, as a bearer, is held to token:admin and may.
    let res = env
        .client
        .post(format!(
            "{}/ui/api/workspaces/{}/agents",
            env.base, ada.workspace.0
        ))
        .bearer_auth(&token)
        .json(&json!({"handle": "via-bearer"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn a_member_who_is_not_an_admin_cannot_add_an_agent() {
    let env = spawn(false).await;
    let ada = env.sign_up("sub-ada", "ada@first-hour.example").await;
    // Once the workspace has a token:admin holder, admin is holding one.
    env.mint(ada.workspace, ada.member, &[capability::TOKEN_ADMIN])
        .await;
    let other = env
        .store
        .create_member(NewMember {
            workspace_id: ada.workspace,
            handle: "teammate".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let cookie = env.signed_in(ada.workspace, other.id).await;

    let (status, out) = env
        .add_agent(&cookie, ada.workspace, json!({"handle": "not-allowed"}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
    assert_eq!(env.agents(ada.workspace).await, 0);

    // The holder still can, from their own session.
    let (status, _) = env
        .add_agent(&ada.cookie, ada.workspace, json!({"handle": "allowed"}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn a_cross_origin_add_or_a_bad_name_creates_nothing() {
    let env = spawn(false).await;
    let ada = env.sign_up("sub-ada", "ada@first-hour.example").await;

    let res = env
        .client
        .post(format!(
            "{}/ui/api/workspaces/{}/agents",
            env.base, ada.workspace.0
        ))
        .header(reqwest::header::COOKIE, &ada.cookie)
        .header("sec-fetch-site", "cross-site")
        .json(&json!({"handle": "csrf"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let too_long = "x".repeat(41);
    for handle in ["", "has space", too_long.as_str(), "semi;colon", "ünï"] {
        let (status, _) = env
            .add_agent(&ada.cookie, ada.workspace, json!({ "handle": handle }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "handle {handle:?}");
    }
    assert_eq!(env.agents(ada.workspace).await, 0);
}

#[tokio::test]
async fn adding_agents_is_rate_limited_per_workspace() {
    let env = spawn(true).await;
    let ada = env.sign_up("sub-ada", "ada@limit-one.example").await;
    let bob = env.sign_up("sub-bob", "bob@limit-two.example").await;

    // The built-in budget is 20 an hour for one workspace.
    for n in 0..20 {
        let (status, out) = env
            .add_agent(
                &ada.cookie,
                ada.workspace,
                json!({ "handle": format!("agent-{n}") }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "add {n}: {out}");
    }
    let (status, _) = env
        .add_agent(
            &ada.cookie,
            ada.workspace,
            json!({"handle": "one-too-many"}),
        )
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(env.agents(ada.workspace).await, 20);

    // Another workspace has its own budget.
    let (status, _) = env
        .add_agent(&bob.cookie, bob.workspace, json!({"handle": "bobs-agent"}))
        .await;
    assert_eq!(status, StatusCode::CREATED);
}
