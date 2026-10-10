//! The workspace switcher (Open Work Next 6, `docs/Hosted Console.md`): two
//! tenants, and a person who is a member of more than one workspace.
//!
//! - `GET /auth/session/workspaces` lists exactly the workspaces of the
//!   identity this session signed in with, never another identity's, never
//!   one derived from the member, and never past a token session's workspace.
//! - Switching is a fresh sign-in: it ends the previous session, and switching
//!   to a workspace the identity is not a member of creates nothing.
//!
//! Runs with auth enabled and auto-provisioning off, as a hosted instance must.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings, SESSION_DELETE},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewMember, NewOidcIdentity, NewWorkspace, WorkspaceId,
};
use reqwest::{redirect::Policy, StatusCode};
use serde_json::Value;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

const SECRET: &[u8] = b"session-workspaces-e2e-secret-32-bytes!";
const ISSUER: &str = "https://mock.idp.local";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    pool: SqlitePool,
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
            // A hosted instance never provisions on sign-in.
            auto_provision: false,
            link_email: false,
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

    async fn sign_in(&self, ws: WorkspaceId, subject: &str, cookie: Option<&str>) -> String {
        let res = self.sign_in_raw(ws, subject, cookie).await;
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
        session_cookie(&res).expect("a session cookie")
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

    async fn count(&self, sql: &str, ws: WorkspaceId) -> i64 {
        sqlx::query_scalar(sql)
            .bind(ws.0)
            .fetch_one(&self.pool)
            .await
            .unwrap()
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

/// Two tenants: identity `x` is a member of A and B, identity `y` of C and of
/// A (a different member). Returns (A, B, C, x's member in B).
async fn two_tenants(env: &Env) -> (WorkspaceId, WorkspaceId, WorkspaceId, MemberId) {
    let a = env.workspace("Alpha").await;
    let b = env.workspace("Bravo").await;
    let c = env.workspace("Charlie").await;
    let xa = env.member(a, "x").await;
    let xb = env.member(b, "x").await;
    let yc = env.member(c, "y").await;
    let ya = env.member(a, "y").await;
    env.link(a, "x", xa).await;
    env.link(b, "x", xb).await;
    env.link(c, "y", yc).await;
    env.link(a, "y", ya).await;
    (a, b, c, xb)
}

#[tokio::test]
async fn each_identity_lists_only_its_own_workspaces() {
    let env = spawn().await;
    let (a, b, c, _) = two_tenants(&env).await;

    let x = env.sign_in(a, "x", None).await;
    let listed = env.listed(&x).await;
    assert_eq!(
        listed.first(),
        Some(&a),
        "the current workspace comes first"
    );
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.contains(&b));
    assert!(!listed.contains(&c), "x is no member of C");

    let (_, body) = env.list(&x).await;
    let rows = body["workspaces"].as_array().unwrap();
    assert_eq!(rows[0]["current"], true);
    assert_eq!(rows[0]["name"], "Alpha");
    assert_eq!(rows[1]["current"], false);
    assert_eq!(rows[1]["name"], "Bravo");
    assert_eq!(rows[1]["handle"], "x");

    // Y shares workspace A with X, and still sees nothing of B.
    let y = env.sign_in(c, "y", None).await;
    let listed = env.listed(&y).await;
    assert_eq!(listed, vec![c, a]);
    assert!(!listed.contains(&b));
}

/// A member that holds two linked subjects (link-by-email allows it) must not
/// lend one subject's workspaces to the other: the list is keyed on the
/// identity that signed in, never on the member.
#[tokio::test]
async fn the_list_follows_the_identity_not_the_member() {
    let env = spawn().await;
    let (a, b, c, _) = two_tenants(&env).await;
    let d = env.workspace("Delta").await;
    // `z` is linked to X's member in A, and is a member of D on its own.
    let xa: MemberId = env
        .store
        .get_oidc_identity(a, ISSUER, "x")
        .await
        .unwrap()
        .member_id;
    env.link(a, "z", xa).await;
    let zd = env.member(d, "z").await;
    env.link(d, "z", zd).await;

    let x = env.sign_in(a, "x", None).await;
    let listed = env.listed(&x).await;
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.contains(&b));
    assert!(
        !listed.contains(&d),
        "z's workspace leaked to x through the shared member"
    );
    assert!(!listed.contains(&c));

    let z = env.sign_in(a, "z", None).await;
    let listed = env.listed(&z).await;
    assert_eq!(listed, vec![a, d]);
}

#[tokio::test]
async fn a_token_session_lists_only_its_own_workspace_and_a_bearer_is_refused() {
    let env = spawn().await;
    let (a, b, _, _) = two_tenants(&env).await;
    let xa = env
        .store
        .get_oidc_identity(a, ISSUER, "x")
        .await
        .unwrap()
        .member_id;
    let secret = TokenSecret::generate();
    env.store
        .create_api_token(NewApiToken {
            workspace_id: a,
            member_id: xa,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let exchanged = env
        .client
        .post(format!("{}/auth/session/from-token", env.base))
        .bearer_auth(secret.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(exchanged.status(), StatusCode::CREATED);
    let cookie = session_cookie(&exchanged).unwrap();
    let listed = env.listed(&cookie).await;
    assert_eq!(
        listed,
        vec![a],
        "a token proves nothing about the person behind it"
    );
    assert!(!listed.contains(&b));

    let bearer = env
        .client
        .get(format!("{}/auth/session/workspaces", env.base))
        .bearer_auth(secret.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(bearer.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_deactivated_or_frozen_member_drops_its_workspace() {
    let env = spawn().await;
    let (a, b, _, xb) = two_tenants(&env).await;
    let x = env.sign_in(a, "x", None).await;
    assert_eq!(env.listed(&x).await, vec![a, b]);

    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO maidan_scim_users (member_id, workspace_id, external_id, active, created_at, updated_at)
         VALUES (?, ?, NULL, 0, ?, ?)",
    )
    .bind(xb.0)
    .bind(b.0)
    .bind(now)
    .bind(now)
    .execute(&env.pool)
    .await
    .unwrap();
    assert_eq!(env.listed(&x).await, vec![a], "SCIM active=false drops B");

    sqlx::query("UPDATE maidan_scim_users SET active = 1 WHERE member_id = ?")
        .bind(xb.0)
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(env.listed(&x).await, vec![a, b]);
    sqlx::query(
        "INSERT INTO maidan_member_freezes (member_id, frozen_at, frozen_by, reason)
         VALUES (?, ?, ?, 'test')",
    )
    .bind(xb.0)
    .bind(now)
    .bind(xb.0)
    .execute(&env.pool)
    .await
    .unwrap();
    assert_eq!(env.listed(&x).await, vec![a], "a frozen member drops B");
}

#[tokio::test]
async fn switching_signs_in_again_and_ends_the_previous_session() {
    let env = spawn().await;
    let (a, b, _, xb) = two_tenants(&env).await;
    let in_a = env.sign_in(a, "x", None).await;
    let in_b = env.sign_in(b, "x", Some(&in_a)).await;

    let session: Value = env
        .client
        .get(format!("{}/auth/session", env.base))
        .header(reqwest::header::COOKIE, &in_b)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["workspace_id"], b.0.to_string());
    assert_eq!(session["member_id"], xb.0.to_string());
    assert_eq!(env.listed(&in_b).await, vec![b, a]);

    assert_eq!(
        env.session_status(&in_a).await,
        StatusCode::UNAUTHORIZED,
        "the session in A is not left live beside the new one"
    );
    let ended: Vec<_> = env
        .store
        .list_audit_for_workspace(a, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.action == SESSION_DELETE)
        .collect();
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert_eq!(ended[0].metadata["reason"], "switched");
}

#[tokio::test]
async fn switching_to_a_workspace_you_are_not_in_creates_nothing() {
    let env = spawn().await;
    let (a, _, c, _) = two_tenants(&env).await;
    let in_a = env.sign_in(a, "x", None).await;
    let members_before = env
        .count(
            "SELECT COUNT(*) FROM maidan_members WHERE workspace_id = ?",
            c,
        )
        .await;

    let refused = env.sign_in_raw(c, "x", Some(&in_a)).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(session_cookie(&refused).is_none(), "no session for C");
    assert_eq!(
        env.count(
            "SELECT COUNT(*) FROM maidan_members WHERE workspace_id = ?",
            c
        )
        .await,
        members_before,
        "no member created in C"
    );
    assert_eq!(
        env.count(
            "SELECT COUNT(*) FROM maidan_oidc_identities WHERE workspace_id = ? AND subject = 'x'",
            c
        )
        .await,
        0,
        "no identity row in C"
    );
    assert_eq!(
        env.count(
            "SELECT COUNT(*) FROM maidan_sessions WHERE workspace_id = ?",
            c
        )
        .await,
        0
    );
    // The refused switch leaves the current session as it was.
    assert_eq!(env.session_status(&in_a).await, StatusCode::OK);
    assert!(!env.listed(&in_a).await.contains(&c));
}
