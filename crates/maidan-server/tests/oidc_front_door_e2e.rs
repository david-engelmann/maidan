//! The front door (`docs/Hosted Console.md`, open questions 4 and 5): signing
//! in without a workspace id, and a login that tells nobody which workspace
//! ids exist.
//!
//! - `GET /auth/oidc/login` with no `workspace_id` signs the identity in to its
//!   most recently used workspace, hints the console to offer the chooser when
//!   it has more, and creates no session when it has none. It only reaches a
//!   workspace where the same issuer and subject already have an identity row:
//!   it never links by email and never provisions.
//! - A real workspace id and an unknown one get the same login redirect, and
//!   the same refusal at the callback.
//!
//! Two tenants throughout: nobody lands in, or is told about, a workspace they
//! are not a member of.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewMember, NewOidcIdentity, NewWorkspace, WorkspaceId};
use reqwest::{redirect::Policy, StatusCode};
use serde_json::Value;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

const SECRET: &[u8] = b"oidc-front-door-e2e-secret-32-bytes!!!!";
const ISSUER: &str = "https://mock.idp.local";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    pool: SqlitePool,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Env {
    spawn_with(false, false).await
}

/// A server on the mock provider, with auto-provisioning and link-by-email as
/// given.
async fn spawn_with(auto_provision: bool, link_email: bool) -> Env {
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
        false, // auth ENABLED
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
            auto_provision,
            link_email,
            session_ttl_secs: 3600,
            pending_ttl_secs: 600,
            cookie_secure: false,
            post_logout_redirect_uri: None,
            first_admin_mint: true,
            auto_mint: false,
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
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Env {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        store,
        pool,
        _dir: dir,
    }
}

impl Env {
    async fn workspace(&self, name: &str) -> WorkspaceId {
        self.store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap()
            .id
    }

    async fn member(&self, ws: WorkspaceId, handle: &str) -> MemberId {
        self.store
            .create_member(NewMember {
                workspace_id: ws,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap()
            .id
    }

    /// Link `subject` to `member` in `ws`, as a first sign-in would.
    async fn link(&self, ws: WorkspaceId, subject: &str, member: MemberId) {
        self.store
            .upsert_oidc_identity(NewOidcIdentity {
                workspace_id: ws,
                issuer: ISSUER.into(),
                subject: subject.into(),
                member_id: member,
                email: None,
            })
            .await
            .unwrap();
    }

    /// Sign `subject` in to `ws` through the mock provider, sending `cookie`
    /// (this browser's current session) if any. The callback's response.
    async fn sign_in_raw(
        &self,
        ws: WorkspaceId,
        subject: &str,
        cookie: Option<&str>,
    ) -> reqwest::Response {
        let login = self
            .client
            .get(format!(
                "{}/auth/oidc/login?workspace_id={}",
                self.base, ws.0
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
        let location = login.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap()
            .replace("mock_sub=mock-user", &format!("mock_sub={subject}"));
        let mut req = self.client.get(format!("{}{location}", self.base));
        if let Some(cookie) = cookie {
            req = req.header(reqwest::header::COOKIE, cookie);
        }
        req.send().await.unwrap()
    }

    async fn list(&self, cookie: &str) -> (StatusCode, Value) {
        let res = self
            .client
            .get(format!("{}/auth/session/workspaces", self.base))
            .header(reqwest::header::COOKIE, cookie)
            .send()
            .await
            .unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    /// The listed workspace ids, in order.
    async fn listed(&self, cookie: &str) -> Vec<WorkspaceId> {
        let (status, body) = self.list(cookie).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| WorkspaceId(w["workspace_id"].as_str().unwrap().parse().unwrap()))
            .collect()
    }

    async fn session_status(&self, cookie: &str) -> StatusCode {
        self.client
            .get(format!("{}/auth/session", self.base))
            .header(reqwest::header::COOKIE, cookie)
            .send()
            .await
            .unwrap()
            .status()
    }
}

fn session_cookie(res: &reqwest::Response) -> Option<String> {
    res.headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|s| s.split(';').next())
        .find(|p| p.starts_with("maidan_session=") && p.len() > "maidan_session=".len())
        .map(str::to_string)
}

impl Env {
    /// Start a front-door login (no workspace id) and finish it at the mock
    /// provider as `subject`, sending `cookie` if any.
    async fn front_door_raw(&self, subject: &str, cookie: Option<&str>) -> reqwest::Response {
        let login = self
            .client
            .get(format!("{}/auth/oidc/login?return_to=/ui/", self.base))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
        let location = login.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap()
            .replace("mock_sub=mock-user", &format!("mock_sub={subject}"));
        let mut req = self.client.get(format!("{}{location}", self.base));
        if let Some(cookie) = cookie {
            req = req.header(reqwest::header::COOKIE, cookie);
        }
        req.send().await.unwrap()
    }

    /// The session's `(workspace_id, member_id)`.
    async fn whoami(&self, cookie: &str) -> (WorkspaceId, MemberId) {
        let res = self
            .client
            .get(format!("{}/auth/session", self.base))
            .header(reqwest::header::COOKIE, cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: Value = res.json().await.unwrap();
        (
            WorkspaceId(body["workspace_id"].as_str().unwrap().parse().unwrap()),
            MemberId(body["member_id"].as_str().unwrap().parse().unwrap()),
        )
    }

    async fn total(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.unwrap()
    }
}

fn location(res: &reqwest::Response) -> String {
    res.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

/// Two tenants: `x` is a member of A and B and signed in to B last; `y` is a
/// member of C and A (a different member) and signed in to A last; `z` only
/// of C. Returns (A, B, C, x in B, y in A, z in C).
async fn tenants(
    env: &Env,
) -> (
    WorkspaceId,
    WorkspaceId,
    WorkspaceId,
    MemberId,
    MemberId,
    MemberId,
) {
    let a = env.workspace("Alpha").await;
    let b = env.workspace("Bravo").await;
    let c = env.workspace("Charlie").await;
    let xa = env.member(a, "x").await;
    let xb = env.member(b, "x").await;
    let yc = env.member(c, "y").await;
    let ya = env.member(a, "y").await;
    let zc = env.member(c, "z").await;
    env.link(a, "x", xa).await;
    env.link(c, "y", yc).await;
    env.link(c, "z", zc).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    env.link(b, "x", xb).await;
    env.link(a, "y", ya).await;
    (a, b, c, xb, ya, zc)
}

#[tokio::test]
async fn the_front_door_signs_in_to_the_latest_workspace_and_offers_the_rest() {
    let env = spawn().await;
    let (a, b, c, xb, ya, _) = tenants(&env).await;

    let res = env.front_door_raw("x", None).await;
    assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(location(&res), "/ui/?choose_workspace=1");
    let x = session_cookie(&res).expect("a session");
    assert_eq!(env.whoami(&x).await, (b, xb), "x signed in to B last");
    let listed = env.listed(&x).await;
    assert_eq!(listed, vec![b, a]);
    assert!(!listed.contains(&c), "x is no member of C");

    // Y shares A with X, lands in A as Y's own member, and sees nothing of B.
    let res = env.front_door_raw("y", None).await;
    assert_eq!(location(&res), "/ui/?choose_workspace=1");
    let y = session_cookie(&res).expect("a session");
    assert_eq!(env.whoami(&y).await, (a, ya));
    assert_eq!(env.listed(&y).await, vec![a, c]);

    // The front door is a sign-in like any other: it ends this browser's
    // previous session.
    let again = session_cookie(&env.front_door_raw("x", Some(&x)).await).expect("a session");
    assert_ne!(again, x);
    assert_eq!(env.session_status(&x).await, StatusCode::UNAUTHORIZED);
    assert_eq!(env.session_status(&again).await, StatusCode::OK);
}

#[tokio::test]
async fn one_workspace_goes_straight_in() {
    let env = spawn().await;
    let (_, _, c, _, _, zc) = tenants(&env).await;
    let res = env.front_door_raw("z", None).await;
    assert_eq!(location(&res), "/ui/", "no chooser for one workspace");
    let z = session_cookie(&res).expect("a session");
    assert_eq!(env.whoami(&z).await, (c, zc));
}

#[tokio::test]
async fn no_workspace_gets_no_session_and_creates_nothing() {
    // Even with auto-provisioning and link-by-email on: the front door has no
    // workspace to provision in, and a member whose handle is the person's
    // verified email elsewhere is not theirs until that workspace's own
    // sign-in links it.
    let env = spawn_with(true, true).await;
    let (_, _, c, _, _, _) = tenants(&env).await;
    env.member(c, "human@example.com").await;
    let members = env.total("SELECT COUNT(*) FROM maidan_members").await;
    let identities = env
        .total("SELECT COUNT(*) FROM maidan_oidc_identities")
        .await;

    let res = env.front_door_raw("nobody", None).await;
    assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(location(&res), "/ui/?no_workspace=1");
    assert!(session_cookie(&res).is_none(), "no session");
    assert_eq!(env.total("SELECT COUNT(*) FROM maidan_sessions").await, 0);
    assert_eq!(
        env.total("SELECT COUNT(*) FROM maidan_members").await,
        members
    );
    assert_eq!(
        env.total("SELECT COUNT(*) FROM maidan_oidc_identities")
            .await,
        identities
    );
}

#[tokio::test]
async fn the_front_door_skips_a_deactivated_or_frozen_member() {
    let env = spawn().await;
    let (a, b, _, xb, _, _) = tenants(&env).await;
    let xa = env
        .store
        .get_oidc_identity(a, ISSUER, "x")
        .await
        .unwrap()
        .member_id;
    env.store
        .create_scim_user(xb, b, Some("ext-xb"), false)
        .await
        .unwrap();
    let res = env.front_door_raw("x", None).await;
    assert_eq!(location(&res), "/ui/", "only A is left, so no chooser");
    let x = session_cookie(&res).expect("a session");
    assert_eq!(env.whoami(&x).await, (a, xa));

    let admin = env.member(a, "admin").await;
    env.store.freeze_member(xa, admin, None).await.unwrap();
    let res = env.front_door_raw("x", None).await;
    assert_eq!(location(&res), "/ui/?no_workspace=1");
    assert!(session_cookie(&res).is_none());
}

/// The login redirect with its one-time `state` (and nothing else) masked.
fn masked_login(res: &reqwest::Response) -> String {
    let location = location(res);
    let (path, query) = location.split_once('?').unwrap();
    let query: Vec<String> = query
        .split('&')
        .map(|kv| match kv.split_once('=') {
            Some(("state", _)) => "state=*".to_string(),
            _ => kv.to_string(),
        })
        .collect();
    format!("{path}?{}", query.join("&"))
}

#[tokio::test]
async fn a_real_and_an_unknown_workspace_id_get_identical_responses() {
    let env = spawn().await;
    let (_, _, c, _, _, _) = tenants(&env).await;
    let unknown = WorkspaceId(uuid::Uuid::new_v4());

    // Before authentication: the same redirect, the same headers, the same
    // (empty) body.
    let mut seen = Vec::new();
    for ws in [c, unknown] {
        let res = env
            .client
            .get(format!(
                "{}/auth/oidc/login?workspace_id={}",
                env.base, ws.0
            ))
            .send()
            .await
            .unwrap();
        let status = res.status();
        let mut names: Vec<String> = res.headers().keys().map(|k| k.to_string()).collect();
        names.sort();
        names.retain(|n| n != "date");
        let masked = masked_login(&res);
        let body = res.text().await.unwrap();
        seen.push((status, names, masked, body));
    }
    assert_eq!(seen[0].0, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(seen[0], seen[1], "a real and an unknown id look the same");

    // After authentication as x, who is no member of either: the same refusal,
    // and nothing created.
    let members = env.total("SELECT COUNT(*) FROM maidan_members").await;
    let mut refusals = Vec::new();
    for ws in [c, unknown] {
        let res = env.sign_in_raw(ws, "x", None).await;
        assert!(session_cookie(&res).is_none());
        let status = res.status();
        let content_type = res.headers()[reqwest::header::CONTENT_TYPE].clone();
        refusals.push((status, content_type, res.text().await.unwrap()));
    }
    assert_eq!(refusals[0].0, StatusCode::FORBIDDEN, "{refusals:?}");
    assert_eq!(
        refusals[0], refusals[1],
        "the callback tells nobody which id is real"
    );
    assert_eq!(
        env.total("SELECT COUNT(*) FROM maidan_members").await,
        members
    );
    assert_eq!(env.total("SELECT COUNT(*) FROM maidan_sessions").await, 0);
}

#[tokio::test]
async fn a_workspace_sign_in_still_works() {
    let env = spawn().await;
    let (a, _, _, _, _, _) = tenants(&env).await;
    let xa = env
        .store
        .get_oidc_identity(a, ISSUER, "x")
        .await
        .unwrap()
        .member_id;
    let res = env.sign_in_raw(a, "x", None).await;
    assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(location(&res), "/ui/", "a chosen workspace gets no chooser");
    let x = session_cookie(&res).expect("a session");
    assert_eq!(env.whoami(&x).await, (a, xa));
}

#[tokio::test]
async fn an_unknown_workspace_id_is_refused_even_where_sign_in_provisions() {
    // Login no longer checks the id, so the callback must: with
    // auto-provisioning on it would otherwise try to create a member in a
    // workspace that doesn't exist.
    let env = spawn_with(true, true).await;
    tenants(&env).await;
    let members = env.total("SELECT COUNT(*) FROM maidan_members").await;
    let res = env
        .sign_in_raw(WorkspaceId(uuid::Uuid::new_v4()), "x", None)
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(session_cookie(&res).is_none());
    let body: Value = res.json().await.unwrap();
    assert!(
        body.to_string()
            .contains("not provisioned in this workspace"),
        "{body}"
    );
    assert_eq!(
        env.total("SELECT COUNT(*) FROM maidan_members").await,
        members
    );
    assert_eq!(env.total("SELECT COUNT(*) FROM maidan_sessions").await, 0);
}
