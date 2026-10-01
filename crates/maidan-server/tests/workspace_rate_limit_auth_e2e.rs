//! The workspace rate limit is charged only for a caller authenticated into
//! that workspace, and an unverified bearer shares the client IP bucket.
//! Its own binary so the limits it sets do not leak into other tests.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
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

fn set_limits(global: &str, workspace: &str) {
    unsafe {
        std::env::set_var("MAIDAN_RATE_LIMIT_MAX", global);
        std::env::set_var("MAIDAN_RATE_LIMIT_WINDOW_SECS", "60");
        std::env::set_var("MAIDAN_WORKSPACE_RATE_LIMIT_MAX", workspace);
        std::env::set_var("MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS", "60");
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

async fn seed(store: &dyn Store, name: &str, handle: &str) -> (String, String) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: name.to_string(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: handle.to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.to_string()],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws.id.to_string(), secret.as_str().to_string())
}

async fn get_workspace(h: &Harness, wid: &str, bearer: Option<&str>) -> StatusCode {
    let mut req = h.client.get(format!("{}/workspaces/{wid}", h.base()));
    if let Some(bearer) = bearer {
        req = req.header("Authorization", format!("Bearer {bearer}"));
    }
    req.send().await.expect("get workspace").status()
}

#[tokio::test]
async fn foreign_and_invented_callers_do_not_spend_the_workspace_budget() {
    // Global off, so this phase sees only the workspace budget of 2.
    set_limits("0", "2");
    let h = spawn().await;
    let (a, token_a) = seed(h.store.as_ref(), "a", "agent-a").await;
    let (_b, token_b) = seed(h.store.as_ref(), "b", "agent-b").await;

    for _ in 0..5 {
        assert_eq!(
            get_workspace(&h, &a, Some(&token_b)).await,
            StatusCode::FORBIDDEN,
            "a token for another workspace is refused without spending A's budget"
        );
    }
    for _ in 0..5 {
        assert_eq!(
            get_workspace(&h, &a, None).await,
            StatusCode::UNAUTHORIZED,
            "no credential does not spend A's budget"
        );
    }
    for i in 0..5 {
        assert_eq!(
            get_workspace(&h, &a, Some(&format!("invented-{i}"))).await,
            StatusCode::UNAUTHORIZED,
            "an invented bearer does not spend A's budget"
        );
    }

    assert_eq!(get_workspace(&h, &a, Some(&token_a)).await, StatusCode::OK);
    assert_eq!(get_workspace(&h, &a, Some(&token_a)).await, StatusCode::OK);
    assert_eq!(
        get_workspace(&h, &a, Some(&token_a)).await,
        StatusCode::TOO_MANY_REQUESTS,
        "A's own third request is the one that meets the budget of 2"
    );

    // The invented bearers share one IP bucket of 2. A's real token keeps
    // its own, so it is not locked out by them.
    set_limits("2", "0");
    assert_eq!(
        get_workspace(&h, &a, Some("made-up-one")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get_workspace(&h, &a, Some("made-up-two")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get_workspace(&h, &a, Some("made-up-three")).await,
        StatusCode::TOO_MANY_REQUESTS,
        "a third invented bearer from the same IP is rate limited, not given a new bucket"
    );
    assert_eq!(
        get_workspace(&h, &a, Some(&token_a)).await,
        StatusCode::OK,
        "a verified bearer still has its own bucket after the IP budget is spent"
    );
    assert_eq!(
        get_workspace(&h, &a, Some("made-up-four")).await,
        StatusCode::TOO_MANY_REQUESTS
    );

    h.shutdown().await;
}
