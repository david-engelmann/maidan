//! RED-005: one credential cannot hold open event streams without bound.
//!
//! A stream answers at once and stays open, so neither the in-flight ceiling
//! nor the rate limiter bounds how many one token holds. Past its ceiling a
//! credential's next stream, on any stream route, is refused with `429`;
//! another token, in the same workspace or another, still streams; and a
//! closed stream gives its place back. `maidan_sse_streams` counts what is
//! open.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{expand_set, hash_secret, TokenSecret, AGENT_WORKER};
use maidan_bus::InMemoryBus;
use maidan_server::{router, stream_limit::StreamLimit, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;

const CEILING: usize = 4;

struct Harness {
    base: String,
    state: AppState,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Harness {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(8)
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
    let mut state = AppState::for_tests(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(256)),
        search,
    );
    state.auth_disabled = false;
    state.test_identity_header = false;
    state.stream_limit = StreamLimit::new(CEILING);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        state,
        // A connection per stream: a held stream keeps its connection busy.
        client: reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap(),
        _dir: dir,
    }
}

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

async fn worker_token(store: &dyn Store, ws: WorkspaceId, handle: &str) -> String {
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: expand_set(AGENT_WORKER).unwrap(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

impl Harness {
    /// Open a stream and keep it open by holding the response unread.
    async fn open(&self, path: &str, bearer: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap()
    }

    async fn open_streams_become(&self, expected: usize) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if self.state.stream_limit.open_streams() == expected {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.state.stream_limit.open_streams() == expected
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_credential_cannot_hold_more_open_streams_than_its_ceiling() {
    let h = spawn().await;
    let store = h.state.store.clone();
    let a = workspace(store.as_ref(), "a").await;
    let b = workspace(store.as_ref(), "b").await;
    let greedy = worker_token(store.as_ref(), a, "greedy").await;
    let colleague = worker_token(store.as_ref(), a, "colleague").await;
    let other_tenant = worker_token(store.as_ref(), b, "other").await;

    // The ceiling is per credential, not per route: these are four routes.
    let routes = [
        "/mcp/notifications".to_string(),
        "/mcp/stream".to_string(),
        format!("/agui/stream?workspace_id={}", a.0),
        "/mcp/notifications".to_string(),
    ];
    let mut held = Vec::new();
    for path in &routes {
        let resp = h.open(path, &greedy).await;
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        held.push(resp);
    }
    assert_eq!(h.state.stream_limit.open_streams(), CEILING);

    for path in [
        "/mcp/notifications".to_string(),
        "/mcp/stream".to_string(),
        format!("/agui/stream?workspace_id={}", a.0),
    ] {
        let refused = h.open(&path, &greedy).await;
        assert_eq!(
            refused.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "{path}: stream {} of a credential at its ceiling of {CEILING}",
            CEILING + 1
        );
    }

    let same_workspace = h.open("/mcp/notifications", &colleague).await;
    assert_eq!(same_workspace.status(), StatusCode::OK, "another token");
    let other_workspace = h.open("/mcp/notifications", &other_tenant).await;
    assert_eq!(other_workspace.status(), StatusCode::OK, "another tenant");
    assert_eq!(h.state.stream_limit.open_streams(), CEILING + 2);

    // Closing a stream gives its place back.
    drop(held.pop());
    assert!(
        h.open_streams_become(CEILING + 1).await,
        "a closed stream still counts: {} open",
        h.state.stream_limit.open_streams()
    );
    let reopened = h.open("/mcp/notifications", &greedy).await;
    assert_eq!(reopened.status(), StatusCode::OK, "reopen after a close");

    drop((held, reopened, same_workspace, other_workspace));
    assert!(
        h.open_streams_become(0).await,
        "streams still counted after every client left: {}",
        h.state.stream_limit.open_streams()
    );
}
