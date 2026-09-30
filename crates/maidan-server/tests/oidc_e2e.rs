//! OIDC login/callback/logout with deterministic mock IdP.

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
use maidan_types::{NewMember, NewWorkspace, WorkspaceId};
use reqwest::{redirect::Policy, StatusCode};
use sqlx::sqlite::SqlitePoolOptions;

const TEST_SESSION_SECRET: &[u8] = b"test-session-secret-32-bytes-min!";

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    workspace_id: WorkspaceId,
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

fn mock_oidc_settings(auto_mint: bool) -> OidcSettings {
    OidcSettings {
        enabled: true,
        mock: true,
        issuer: "https://mock.idp.local".into(),
        redirect_uri: "http://127.0.0.1/auth/oidc/callback".into(),
        auto_provision: true,
        link_email: false,
        session_ttl_secs: 3600,
        pending_ttl_secs: 600,
        cookie_secure: false,
        post_logout_redirect_uri: None,
        first_admin_mint: true,
        auto_mint,
    }
}

async fn spawn_with_settings(settings: OidcSettings) -> Harness {
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
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "oidc-test".into(),
        })
        .await
        .unwrap();
    store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "existing".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();

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
        true,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.oidc = Some(Arc::new(OidcRuntime {
        settings,
        session_secret: Arc::from(TEST_SESSION_SECRET),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    Harness {
        addr,
        server,
        client,
        workspace_id: workspace.id,
        store,
        _dir: dir,
    }
}

async fn spawn() -> Harness {
    spawn_with_settings(mock_oidc_settings(false)).await
}

/// The discovery document tells a first-run screen that browser sign-in works
/// here, and where it starts.
#[tokio::test]
async fn discovery_says_oidc_sign_in_is_available() {
    let h = spawn().await;
    let body: serde_json::Value = h
        .client
        .get(format!("{}/.well-known/maidan.json", h.base()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["auth"]["oidc"], true);
    assert_eq!(body["auth"]["oidc_login"], "/auth/oidc/login");
}

#[tokio::test]
async fn mock_oidc_login_sets_session_cookie_and_logout_clears_it() {
    let h = spawn().await;
    let base = h.base();
    let wid = h.workspace_id.0;

    let login = h
        .client
        .get(format!("{base}/auth/oidc/login?workspace_id={wid}"))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = login
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let callback = h
        .client
        .get(format!("{base}{location}"))
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::TEMPORARY_REDIRECT);
    let cookies = callback.headers().get_all(reqwest::header::SET_COOKIE);
    let session_cookie = cookies
        .iter()
        .find_map(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .next()
                .filter(|p| p.starts_with("maidan_session="))
        })
        .expect("session cookie");
    let cookie_payload = session_cookie
        .trim_start_matches("maidan_session=")
        .split(';')
        .next()
        .unwrap();
    assert!(cookie_payload.contains('.'));

    let session_res = h
        .client
        .get(format!("{base}/auth/session"))
        .header(reqwest::header::COOKIE, session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(session_res.status(), StatusCode::OK);
    let body: serde_json::Value = session_res.json().await.unwrap();
    assert_eq!(body["workspace_id"].as_str().unwrap(), wid.to_string());

    let mint = h
        .client
        .post(format!("{base}/auth/session/mint"))
        .header(reqwest::header::COOKIE, session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(mint.status(), StatusCode::CREATED);
    let mint_body: serde_json::Value = mint.json().await.unwrap();
    let secret = mint_body["secret"].as_str().unwrap();

    let events = h
        .client
        .get(format!(
            "{base}/ui/api/workspaces/{wid}/events?after_id=0&limit=10"
        ))
        .header(reqwest::header::COOKIE, session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);

    let mint_again = h
        .client
        .post(format!("{base}/auth/session/mint"))
        .header(reqwest::header::COOKIE, session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(mint_again.status(), StatusCode::FORBIDDEN);

    let _ = secret;

    let logout = h
        .client
        .post(format!("{base}/auth/logout"))
        .header(reqwest::header::COOKIE, session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::SEE_OTHER);
    let cleared = logout
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .any(|v| v.to_str().map(|s| s.contains("Max-Age=0")).unwrap_or(false));
    assert!(cleared);

    h.shutdown().await;
}

#[tokio::test]
async fn mock_oidc_callback_redirects_with_auto_mint_hint_when_enabled() {
    let h = spawn_with_settings(mock_oidc_settings(true)).await;
    let base = h.base();
    let wid = h.workspace_id.0;

    let login = h
        .client
        .get(format!("{base}/auth/oidc/login?workspace_id={wid}"))
        .send()
        .await
        .unwrap();
    let location = login
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let callback = h
        .client
        .get(format!("{base}{location}"))
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::TEMPORARY_REDIRECT);
    let redirect = callback
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        redirect.contains("auto_mint=1"),
        "expected auto_mint hint, got {redirect}"
    );

    h.shutdown().await;
}

impl Harness {
    /// Sign in through the mock IdP and return the session cookie.
    async fn sign_in(&self) -> String {
        let base = self.base();
        let login = self
            .client
            .get(format!(
                "{base}/auth/oidc/login?workspace_id={}",
                self.workspace_id.0
            ))
            .send()
            .await
            .unwrap();
        let location = login.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        let callback = self
            .client
            .get(format!("{base}{location}"))
            .send()
            .await
            .unwrap();
        assert_eq!(callback.status(), StatusCode::TEMPORARY_REDIRECT);
        callback
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|s| {
                s.split(';')
                    .next()
                    .filter(|p| p.starts_with("maidan_session="))
            })
            .expect("session cookie")
            .to_string()
    }

    async fn audit(&self, action: &str) -> Vec<maidan_types::AuditEvent> {
        let mut rows = self
            .store
            .list_audit_for_workspace(self.workspace_id, 100)
            .await
            .unwrap();
        rows.retain(|row| row.action == action);
        rows
    }
}

/// Signing in creates a credential and may provision a member; signing out
/// ends one. Both went around the request layer and left no record. Each now
/// writes its row in the session write's own transaction, naming the member.
#[tokio::test]
async fn signing_in_and_out_are_recorded_with_the_member_as_actor() {
    let h = spawn().await;

    let cookie = h.sign_in().await;
    let created = h.audit(maidan_server::oidc::SESSION_CREATE).await;
    assert_eq!(created.len(), 1, "{created:?}");
    let member = created[0].actor_id.expect("the member is the actor");
    assert_eq!(created[0].subject_id, Some(member));
    assert_eq!(created[0].target_id, Some(member.0));
    assert_eq!(created[0].metadata["member"], "provisioned");
    assert_eq!(created[0].metadata["issuer"], "https://mock.idp.local");
    assert_eq!(
        h.store.get_member(member).await.unwrap().workspace_id,
        h.workspace_id
    );

    // The same identity again finds its member, and says so.
    let second = h.sign_in().await;
    let created = h.audit(maidan_server::oidc::SESSION_CREATE).await;
    assert_eq!(created.len(), 2);
    assert!(created
        .iter()
        .any(|row| row.metadata["member"] == "existing" && row.actor_id == Some(member)));

    let logout = |cookie: String| {
        h.client
            .post(format!("{}/auth/logout", h.base()))
            .header(reqwest::header::COOKIE, cookie)
            .send()
    };
    assert_eq!(
        logout(cookie.clone()).await.unwrap().status(),
        StatusCode::TEMPORARY_REDIRECT
    );
    let deleted = h.audit(maidan_server::oidc::SESSION_DELETE).await;
    assert_eq!(deleted.len(), 1, "{deleted:?}");
    assert_eq!(deleted[0].actor_id, Some(member));
    let session = h
        .client
        .get(format!("{}/auth/session", h.base()))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::UNAUTHORIZED);

    // Signing out of a session already ended ends nothing and records nothing.
    assert_eq!(
        logout(cookie).await.unwrap().status(),
        StatusCode::TEMPORARY_REDIRECT
    );
    assert_eq!(h.audit(maidan_server::oidc::SESSION_DELETE).await.len(), 1);
    let _ = second;
    h.shutdown().await;
}
