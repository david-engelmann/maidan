//! A workspace display name can be set after bootstrap.
//!
//! `POST /workspaces` is closed once a workspace exists. `PATCH /workspaces/:id`
//! is the supported rename, for a bearer and for a signed-in session through
//! `/ui/api`. `GET /auth/session` returns the member display name so the page
//! does not have to show the member id.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, session::SessionSettings, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::{header, StatusCode};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const SESSION_SECRET: &[u8] = b"test-session-secret-32-bytes-min!";

struct Harness {
    addr: SocketAddr,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    fn origin(&self) -> String {
        format!("http://{}", self.addr)
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
    state.sessions = Some(SessionSettings {
        secret: Arc::from(SESSION_SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        addr,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        server,
        store,
        _dir: dir,
    }
}

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: &[&str]) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn patch_names_a_workspace_and_the_session_returns_the_display_name() {
    let h = spawn().await;
    let ws = h
        .store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let other = h
        .store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .unwrap();
    let member = h
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "river".into(),
            display_name: Some("River Chen".into()),
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let writer = mint(
        h.store.as_ref(),
        ws.id,
        member.id,
        &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE],
    )
    .await;
    let reader = mint(
        h.store.as_ref(),
        ws.id,
        member.id,
        &[capability::WORKSPACE_READ],
    )
    .await;

    let blank = h
        .client
        .patch(h.url(&format!("/workspaces/{}", ws.id.0)))
        .bearer_auth(&writer)
        .json(&json!({ "name": "   " }))
        .send()
        .await
        .unwrap();
    assert_eq!(blank.status(), StatusCode::BAD_REQUEST);

    let long = "n".repeat(201);
    let too_long = h
        .client
        .patch(h.url(&format!("/workspaces/{}", ws.id.0)))
        .bearer_auth(&writer)
        .json(&json!({ "name": long }))
        .send()
        .await
        .unwrap();
    assert_eq!(too_long.status(), StatusCode::BAD_REQUEST);

    let denied = h
        .client
        .patch(h.url(&format!("/workspaces/{}", ws.id.0)))
        .bearer_auth(&reader)
        .json(&json!({ "name": "North Room" }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denied_body = denied.text().await.unwrap();
    assert!(
        denied_body.contains("missing capability: workspace:write"),
        "{denied_body}"
    );

    let foreign = h
        .client
        .patch(h.url(&format!("/workspaces/{}", other.id.0)))
        .bearer_auth(&writer)
        .json(&json!({ "name": "North Room" }))
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);

    let renamed = h
        .client
        .patch(h.url(&format!("/workspaces/{}", ws.id.0)))
        .bearer_auth(&writer)
        .json(&json!({ "name": "  North Room  " }))
        .send()
        .await
        .unwrap();
    assert_eq!(renamed.status(), StatusCode::OK);
    let body: Value = renamed.json().await.unwrap();
    assert_eq!(body["name"], json!("North Room"));
    assert_eq!(body["id"], json!(ws.id.0));

    let got = h
        .client
        .get(h.url(&format!("/workspaces/{}", ws.id.0)))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), StatusCode::OK);
    let got: Value = got.json().await.unwrap();
    assert_eq!(got["name"], json!("North Room"));

    let exchange = h
        .client
        .post(h.url("/auth/session/from-token"))
        .bearer_auth(&writer)
        .send()
        .await
        .unwrap();
    assert_eq!(exchange.status(), StatusCode::CREATED);
    let cookie = exchange
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let created: Value = exchange.json().await.unwrap();
    assert_eq!(created["display_name"], json!("River Chen"));
    assert_eq!(created["member_id"], json!(member.id.0));

    let session = h
        .client
        .get(h.url("/auth/session"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::OK);
    let session: Value = session.json().await.unwrap();
    assert_eq!(session["display_name"], json!("River Chen"));
    assert_eq!(session["member_id"], json!(member.id.0));

    let via_ui = h
        .client
        .patch(h.url(&format!("/ui/api/workspaces/{}", ws.id.0)))
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, h.origin())
        .json(&json!({ "name": "South Room" }))
        .send()
        .await
        .unwrap();
    let via_status = via_ui.status();
    let via_raw = via_ui.text().await.unwrap();
    assert_eq!(via_status, StatusCode::OK, "{via_raw}");
    let via_ui: Value = serde_json::from_str(&via_raw).unwrap();
    assert_eq!(via_ui["name"], json!("South Room"));

    h.server.abort();
}
