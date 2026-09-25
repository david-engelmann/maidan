//! A withdrawal over HTTP, with and without a legal hold (auth on). Once a
//! member withdraws a message, its earlier versions are gone from every
//! member's view, held or not. Under a hold the words are kept, and only an
//! admin reads them, through the audited preserved read. Whether a hold exists
//! is an admin's business too: the custodians it binds do not see it.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Ctx {
    addr: SocketAddr,
    _server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}
impl Ctx {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }
}

async fn spawn() -> Ctx {
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
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
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ctx {
        addr,
        _server: server,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        store,
        _dir: dir,
    }
}

async fn mk_member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id
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
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    format!("Bearer {}", secret.as_str())
}

async fn send(
    ctx: &Ctx,
    method: reqwest::Method,
    token: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = ctx
        .client
        .request(method, format!("{}{path}", ctx.base()))
        .header("Authorization", token);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Post, edit, withdraw, as the member. Returns the message id.
async fn say_then_withdraw(ctx: &Ctx, token: &str, thread: uuid::Uuid, last: &str) -> String {
    use reqwest::Method;
    let (s, posted) = send(
        ctx,
        Method::POST,
        token,
        &format!("/threads/{thread}/messages"),
        Some(json!({"body": "first"})),
    )
    .await;
    assert!(s.is_success(), "post {s}");
    let id = posted["id"].as_str().unwrap().to_string();
    let (s, _) = send(
        ctx,
        Method::PATCH,
        token,
        &format!("/messages/{id}"),
        Some(json!({"body": last})),
    )
    .await;
    assert!(s.is_success(), "edit {s}");
    let (s, _) = send(ctx, Method::DELETE, token, &format!("/messages/{id}"), None).await;
    assert!(s.is_success(), "withdraw {s}");
    id
}

#[tokio::test]
async fn a_withdrawal_withdraws_and_a_hold_keeps_it_for_admins_only() {
    use reqwest::Method;
    let ctx = spawn().await;
    let store = ctx.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap()
        .id;
    let member = mk_member(store, ws, "member").await;
    let admin = mk_member(store, ws, "admin").await;
    let member_t = mint(
        store,
        ws,
        member,
        &[capability::WORKSPACE_READ, capability::MESSAGE_POST],
    )
    .await;
    let admin_t = mint(
        store,
        ws,
        admin,
        &[capability::WORKSPACE_READ, capability::TOKEN_ADMIN],
    )
    .await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap()
        .id
        .0;

    let unheld = say_then_withdraw(&ctx, &member_t, thread, "unheld words").await;
    let (_, edits) = send(
        &ctx,
        Method::GET,
        &member_t,
        &format!("/messages/{unheld}/edits"),
        None,
    )
    .await;
    assert_eq!(edits, json!([]), "a withdrawn message's history is gone");

    let wid = ws.0;
    let holds = format!("/workspaces/{wid}/legal-holds");
    let preserved_url = format!("/workspaces/{wid}/legal-holds/preserved");
    let (s, first) = send(
        &ctx,
        Method::POST,
        &admin_t,
        &holds,
        Some(json!({"reason": "matter 1"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "place");
    let held = say_then_withdraw(&ctx, &member_t, thread, "held words").await;

    // The member sees nothing of it, and nothing of the hold.
    let (_, edits) = send(
        &ctx,
        Method::GET,
        &member_t,
        &format!("/messages/{held}/edits"),
        None,
    )
    .await;
    assert_eq!(edits, json!([]));
    let (s, _) = send(&ctx, Method::GET, &member_t, &holds, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a custodian cannot see the hold");
    let (s, _) = send(&ctx, Method::GET, &member_t, &preserved_url, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // The admin reads what was kept.
    let (s, preserved) = send(&ctx, Method::GET, &admin_t, &preserved_url, None).await;
    assert_eq!(s, StatusCode::OK);
    let preserved = preserved.as_array().unwrap();
    assert_eq!(preserved.len(), 1, "{preserved:?}");
    assert_eq!(preserved[0]["message_id"], json!(held));
    assert_eq!(preserved[0]["body"], "held words");
    assert_eq!(preserved[0]["edits"][0]["body_before"], "first");
    let (s, listed) = send(&ctx, Method::GET, &admin_t, &holds, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(listed[0]["reason"], "matter 1");

    // A second matter. Lifting the first keeps the words; lifting the last
    // disposes of them.
    let (s, second) = send(
        &ctx,
        Method::POST,
        &admin_t,
        &holds,
        Some(json!({"reason": "matter 2"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let lift = |hold: &Value| format!("{holds}/{}", hold["id"].as_str().unwrap());
    let (s, _) = send(&ctx, Method::DELETE, &admin_t, &lift(&first), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, still) = send(&ctx, Method::GET, &admin_t, &preserved_url, None).await;
    assert_eq!(
        still.as_array().unwrap().len(),
        1,
        "matter 2 still holds the words"
    );
    let (s, _) = send(&ctx, Method::DELETE, &admin_t, &lift(&second), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, gone) = send(&ctx, Method::GET, &admin_t, &preserved_url, None).await;
    assert_eq!(gone, json!([]), "the last lift disposed of the words");
    let (s, _) = send(&ctx, Method::DELETE, &admin_t, &lift(&second), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
