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
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{NewMember, NewWorkspace, WorkspaceId};
use reqwest::{redirect::Policy, StatusCode};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    workspace_id: WorkspaceId,
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
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
        store,
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
        settings: OidcSettings {
            enabled: true,
            mock: true,
            issuer: "https://mock.idp.local".into(),
            redirect_uri: "http://127.0.0.1/auth/oidc/callback".into(),
            auto_provision: true,
            link_email: false,
            session_ttl_secs: 3600,
            pending_ttl_secs: 600,
            cookie_secure: false,
        },
        client: None,
        http_client: None,
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
        _dir: dir,
    }
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
    assert!(Uuid::parse_str(session_cookie.trim_start_matches("maidan_session=")).is_ok());

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
    assert_eq!(logout.status(), StatusCode::TEMPORARY_REDIRECT);
    let cleared = logout
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .any(|v| v.to_str().map(|s| s.contains("Max-Age=0")).unwrap_or(false));
    assert!(cleared);

    h.shutdown().await;
}
