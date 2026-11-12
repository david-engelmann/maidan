//! A workspace made by sign-up can invite a second person: the person who
//! signed up invites from the console (`/ui/api/workspaces/{wid}/invites`)
//! without holding a token, and their first admin token carries
//! `workspace:admin`, so it invites on the bearer routes and can mint another
//! token that does.
//!
//! Two tenants throughout: each sign-up gets its own workspace, and nothing
//! one admin holds reaches the other's invites. An invited member, a member
//! who is not an admin, an agent (its worker token, or a session of its own)
//! and a session made from a token are refused and create nothing.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::capability;
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewMaidanSession, NewMember, WorkspaceId};
use reqwest::{header, redirect::Policy, Method, StatusCode};
use serde_json::{json, Value};

const SECRET: &[u8] = b"self-serve-invite-e2e-secret-32-bytes!!";
const ISSUER: &str = "https://mock.idp.local";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

/// A server with open sign-up and the first-admin mint on, as maidan.world
/// runs once sign-up opens.
async fn spawn() -> Env {
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
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
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

fn session_cookie(res: &reqwest::Response) -> String {
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("maidan_session="))
        .map(|v| v.split(';').next().unwrap().to_string())
        .expect("session cookie")
}

impl Env {
    /// Follow the mock provider from `start` back to the callback as
    /// `subject` with `email`.
    async fn through_provider(&self, start: &str, subject: &str, email: &str) -> reqwest::Response {
        let start = self
            .client
            .get(format!("{}{start}", self.base))
            .send()
            .await
            .unwrap();
        assert_eq!(start.status(), StatusCode::TEMPORARY_REDIRECT, "{start:?}");
        let location = start.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .replace("mock_sub=mock-user", &format!("mock_sub={subject}"))
            .replace(
                "mock_email=human@example.com",
                &format!("mock_email={email}"),
            );
        self.client
            .get(format!("{}{location}", self.base))
            .send()
            .await
            .unwrap()
    }

    async fn sign_up(&self, subject: &str, email: &str) -> SignedUp {
        let callback = self
            .through_provider("/auth/oidc/signup", subject, email)
            .await;
        assert_eq!(callback.status(), StatusCode::TEMPORARY_REDIRECT);
        let cookie = session_cookie(&callback);
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

    /// A console request on a session cookie, from this origin.
    async fn ui(
        &self,
        cookie: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .client
            .request(method, format!("{}/ui/api{path}", self.base))
            .header(header::COOKIE, cookie)
            .header("sec-fetch-site", "same-origin");
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    /// A request with a bearer token.
    async fn bearer(
        &self,
        token: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    /// `POST /auth/session/mint`: the first admin token, as the console's
    /// first-run card mints it.
    async fn mint_first_admin(&self, cookie: &str) -> (StatusCode, Value) {
        let res = self
            .client
            .post(format!("{}/auth/session/mint", self.base))
            .header(header::COOKIE, cookie)
            .header(header::ORIGIN, &self.base)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
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

    async fn member(&self, workspace: WorkspaceId, handle: &str, kind: MemberKind) -> MemberId {
        self.store
            .create_member(NewMember {
                workspace_id: workspace,
                handle: handle.into(),
                display_name: None,
                kind,
            })
            .await
            .unwrap()
            .id
    }

    async fn invites(&self, workspace: WorkspaceId) -> usize {
        self.store
            .list_workspace_invites(workspace)
            .await
            .unwrap()
            .len()
    }
}

fn invite_body(email: &str, role: &str) -> Option<Value> {
    Some(json!({"email": email, "role": role}))
}

#[tokio::test]
async fn the_person_who_signed_up_invites_from_the_console() {
    let env = spawn().await;
    let ada = env.sign_up("sub-ada", "ada@self-serve.example").await;
    let invites = format!("/workspaces/{}/invites", ada.workspace.0);

    let (status, out) = env
        .ui(
            &ada.cookie,
            Method::POST,
            &invites,
            invite_body("Teammate@Self-Serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{out}");
    assert_eq!(out["invite"]["workspace_id"], ada.workspace.0.to_string());
    assert_eq!(out["invite"]["email"], "teammate@self-serve.example");
    assert_eq!(out["invite"]["role"], "member");
    assert_eq!(out["invite"]["created_by"], ada.member.0.to_string());
    let secret = out["secret"].as_str().expect("the secret, once");
    assert_eq!(
        out["accept_path"],
        format!("/auth/oidc/login?invite={secret}")
    );

    // The minting is audited as the person's.
    let audit = env
        .store
        .list_audit_for_workspace(ada.workspace, 50)
        .await
        .unwrap();
    let row = audit
        .iter()
        .find(|e| e.action == "workspace_invite.create")
        .expect("an audit row for the invite");
    assert_eq!(row.actor_id, Some(ada.member));

    // The console lists it and revokes it.
    let (status, list) = env.ui(&ada.cookie, Method::GET, &invites, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list.as_array().map(Vec::len), Some(1), "{list}");
    assert!(!list.to_string().contains(secret), "{list}");
    let id = out["invite"]["id"].as_str().unwrap();
    let (status, _) = env
        .ui(
            &ada.cookie,
            Method::DELETE,
            &format!("{invites}/{id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let rows = env
        .store
        .list_workspace_invites(ada.workspace)
        .await
        .unwrap();
    assert!(rows[0].revoked_at.is_some(), "{rows:?}");
}

#[tokio::test]
async fn the_first_admin_token_carries_workspace_admin_and_invites() {
    let env = spawn().await;
    let ada = env.sign_up("sub-ada", "ada@self-serve.example").await;

    let (status, minted) = env.mint_first_admin(&ada.cookie).await;
    assert_eq!(status, StatusCode::CREATED, "{minted}");
    let caps: Vec<String> = serde_json::from_value(minted["capabilities"].clone()).unwrap();
    for cap in [capability::TOKEN_ADMIN, capability::WORKSPACE_ADMIN] {
        assert!(caps.iter().any(|c| c == cap), "{cap} in {caps:?}");
    }
    // Nothing that reads or operates across tenants.
    for cap in [capability::OPERATOR_GLOBAL, capability::AUDIT_READ_GLOBAL] {
        assert!(!caps.iter().any(|c| c == cap), "{cap} not in {caps:?}");
    }
    let admin = minted["secret"].as_str().unwrap().to_string();
    let invites = format!("/workspaces/{}/invites", ada.workspace.0);

    let (status, out) = env
        .bearer(
            &admin,
            Method::POST,
            &invites,
            invite_body("one@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{out}");

    // It can hand invite authority to a token of its own, which invites at
    // the role its own capabilities allow.
    let (status, narrow) = env
        .bearer(
            &admin,
            Method::POST,
            &format!(
                "/workspaces/{}/members/{}/tokens",
                ada.workspace.0, ada.member.0
            ),
            Some(json!({
                "label": "inviter",
                "capabilities": [capability::WORKSPACE_ADMIN, capability::WORKSPACE_READ],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{narrow}");
    let narrow = narrow["secret"].as_str().unwrap().to_string();
    let (status, out) = env
        .bearer(
            &narrow,
            Method::POST,
            &invites,
            invite_body("two@self-serve.example", "viewer"),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{out}");

    // Once the admin token exists, the console still invites: its holder
    // holds `workspace:admin`.
    let (status, out) = env
        .ui(
            &ada.cookie,
            Method::POST,
            &invites,
            invite_body("three@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{out}");
    assert_eq!(env.invites(ada.workspace).await, 3);
}

#[tokio::test]
async fn an_admin_cannot_invite_into_another_workspace() {
    let env = spawn().await;
    let ada = env.sign_up("sub-ada", "ada@self-serve.example").await;
    let bob = env.sign_up("sub-bob", "bob@other-tenant.example").await;
    assert_ne!(ada.workspace, bob.workspace);
    let bobs = format!("/workspaces/{}/invites", bob.workspace.0);

    // Bob has one invite of his own.
    let (status, theirs) = env
        .ui(
            &bob.cookie,
            Method::POST,
            &bobs,
            invite_body("bobs-teammate@other-tenant.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{theirs}");
    let theirs = theirs["invite"]["id"].as_str().unwrap().to_string();

    // Ada's session cannot mint, list or revoke in Bob's workspace.
    let (status, _) = env
        .ui(
            &ada.cookie,
            Method::POST,
            &bobs,
            invite_body("intruder@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = env.ui(&ada.cookie, Method::GET, &bobs, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!body.to_string().contains("bobs-teammate"), "{body}");
    let (status, _) = env
        .ui(
            &ada.cookie,
            Method::DELETE,
            &format!("{bobs}/{theirs}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Nor can her admin token, on either door.
    let (status, minted) = env.mint_first_admin(&ada.cookie).await;
    assert_eq!(status, StatusCode::CREATED, "{minted}");
    let admin = minted["secret"].as_str().unwrap();
    let (status, _) = env
        .bearer(
            admin,
            Method::POST,
            &bobs,
            invite_body("intruder@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = env
        .bearer(
            admin,
            Method::POST,
            &format!("/ui/api{bobs}"),
            invite_body("intruder@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = env
        .bearer(admin, Method::DELETE, &format!("{bobs}/{theirs}"), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Bob's invites are as he left them, and none landed in Ada's.
    let rows = env
        .store
        .list_workspace_invites(bob.workspace)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].revoked_at.is_none(), "{rows:?}");
    assert_eq!(env.invites(ada.workspace).await, 0);

    // Each still invites into their own.
    let (status, _) = env
        .ui(
            &ada.cookie,
            Method::POST,
            &format!("/workspaces/{}/invites", ada.workspace.0),
            invite_body("adas-teammate@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn an_invited_member_or_one_who_is_not_an_admin_cannot_invite() {
    let env = spawn().await;
    let ada = env.sign_up("sub-ada", "ada@self-serve.example").await;
    let invites = format!("/workspaces/{}/invites", ada.workspace.0);

    // Ada invites a member, who joins through the link before anyone holds
    // an admin token: the moment the first-admin rule is open.
    let (status, out) = env
        .ui(
            &ada.cookie,
            Method::POST,
            &invites,
            invite_body("grace@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{out}");
    let joined = env
        .through_provider(
            out["accept_path"].as_str().unwrap(),
            "sub-grace",
            "grace@self-serve.example",
        )
        .await;
    assert!(joined.status().is_redirection(), "{}", joined.status());
    let grace = session_cookie(&joined);

    let (status, body) = env
        .ui(
            &grace,
            Method::POST,
            &invites,
            invite_body("friend@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = env.ui(&grace, Method::GET, &invites, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = env.mint_first_admin(&grace).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Once Ada holds the admin token, a human member who is not an admin is
    // refused too.
    let (status, _) = env.mint_first_admin(&ada.cookie).await;
    assert_eq!(status, StatusCode::CREATED);
    let teammate = env
        .member(ada.workspace, "teammate", MemberKind::Human)
        .await;
    let teammate = env.signed_in(ada.workspace, teammate).await;
    let (status, body) = env
        .ui(
            &teammate,
            Method::POST,
            &invites,
            invite_body("friend@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    assert_eq!(env.invites(ada.workspace).await, 1, "only Ada's invite");
}

#[tokio::test]
async fn an_agent_cannot_invite() {
    let env = spawn().await;
    let ada = env.sign_up("sub-ada", "ada@self-serve.example").await;
    let invites = format!("/workspaces/{}/invites", ada.workspace.0);

    // An agent's own sign-in, before anyone holds an admin token: the
    // first-admin rule never makes an agent the admin.
    let seat = env.member(ada.workspace, "seat", MemberKind::Agent).await;
    let seat = env.signed_in(ada.workspace, seat).await;
    let (status, body) = env
        .ui(
            &seat,
            Method::POST,
            &invites,
            invite_body("friend@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // The worker token "Add an agent" gives it, on either door.
    let (status, added) = env
        .ui(
            &ada.cookie,
            Method::POST,
            &format!("/workspaces/{}/agents", ada.workspace.0),
            Some(json!({"handle": "claude"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    let worker = added["token"]["secret"].as_str().unwrap();
    for path in [invites.clone(), format!("/ui/api{invites}")] {
        let (status, body) = env
            .bearer(
                worker,
                Method::POST,
                &path,
                invite_body("friend@self-serve.example", "member"),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
        assert!(
            body.to_string().contains(capability::WORKSPACE_ADMIN),
            "{path}: {body}"
        );
        let (status, _) = env.bearer(worker, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }

    assert_eq!(env.invites(ada.workspace).await, 0);
}

#[tokio::test]
async fn a_session_made_from_a_token_cannot_invite_from_the_console() {
    let env = spawn().await;
    let ada = env.sign_up("sub-ada", "ada@self-serve.example").await;
    let (status, minted) = env.mint_first_admin(&ada.cookie).await;
    assert_eq!(status, StatusCode::CREATED, "{minted}");
    let admin = minted["secret"].as_str().unwrap();

    // Even the admin token's own session: that holder has the bearer route.
    let exchanged = env
        .client
        .post(format!("{}/auth/session/from-token", env.base))
        .bearer_auth(admin)
        .send()
        .await
        .unwrap();
    assert_eq!(exchanged.status(), StatusCode::CREATED);
    let cookie = session_cookie(&exchanged);
    let invites = format!("/workspaces/{}/invites", ada.workspace.0);
    let (status, body) = env
        .ui(
            &cookie,
            Method::POST,
            &invites,
            invite_body("friend@self-serve.example", "member"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(env.invites(ada.workspace).await, 0);
}
