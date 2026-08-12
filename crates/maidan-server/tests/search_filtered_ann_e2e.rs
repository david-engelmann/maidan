//! Cluster 200 (Arc D): the search route pushes an RBAC channel-deny into the
//! query, so a non-member's private-channel hits are excluded *at the source* —
//! they neither leak nor crowd out the requested `limit` with accessible results.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
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

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
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
        client: reqwest::Client::new(),
        store,
        _dir: dir,
    }
}

fn auth(t: &str) -> String {
    format!("Bearer {t}")
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

impl Ctx {
    async fn make_channel(&self, ws: WorkspaceId, tok: &str, name: &str, private: bool) -> String {
        let ch: Value = self
            .client
            .post(format!("{}/workspaces/{}/channels", self.base(), ws.0))
            .header("Authorization", auth(tok))
            .json(&json!({"name": name, "private": private}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        ch["id"].as_str().unwrap().to_string()
    }

    async fn make_thread(&self, cid: &str, tok: &str) -> String {
        let th: Value = self
            .client
            .post(format!("{}/channels/{cid}/threads", self.base()))
            .header("Authorization", auth(tok))
            .json(&json!({"title": "t"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        th["id"].as_str().unwrap().to_string()
    }

    async fn post(&self, tid: &str, tok: &str, author: MemberId, body: &str) {
        self.client
            .post(format!("{}/threads/{tid}/messages", self.base()))
            .header("Authorization", auth(tok))
            .json(&json!({"author_id": author.0, "body": body}))
            .send()
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn search_excludes_private_channel_hits_and_honors_limit() {
    let ctx = spawn().await;
    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let alice = mk_member(ctx.store.as_ref(), ws.id, "alice").await;
    let mallory = mk_member(ctx.store.as_ref(), ws.id, "mallory").await;
    let alice_tok = mint(
        ctx.store.as_ref(),
        ws.id,
        alice,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
            capability::MESSAGE_POST.into(),
            capability::CHANNEL_ADMIN.into(),
            capability::SEARCH_QUERY.into(),
        ],
    )
    .await;
    let mallory_tok = mint(
        ctx.store.as_ref(),
        ws.id,
        mallory,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::SEARCH_QUERY.into(),
        ],
    )
    .await;

    // Private channel (alice auto-admin; mallory NOT a member): 5 "widget" hits.
    let secret = ctx.make_channel(ws.id, &alice_tok, "secret", true).await;
    let secret_thread = ctx.make_thread(&secret, &alice_tok).await;
    for i in 0..5 {
        ctx.post(
            &secret_thread,
            &alice_tok,
            alice,
            &format!("secret widget {i}"),
        )
        .await;
    }
    // Public channel: 3 "widget" hits.
    let open = ctx.make_channel(ws.id, &alice_tok, "open", false).await;
    let open_thread = ctx.make_thread(&open, &alice_tok).await;
    for i in 0..3 {
        ctx.post(
            &open_thread,
            &alice_tok,
            alice,
            &format!("public widget {i}"),
        )
        .await;
    }

    // Mallory searches "widget" with limit 3. There are 5 private + 3 public
    // matches; the private ones are excluded at the query level, so she gets a
    // FULL page of 3 accessible (public) hits — not a short page the post-filter
    // would have left behind — and none from the private channel.
    let hits: Vec<Value> = ctx
        .client
        .get(format!(
            "{}/workspaces/{}/search?q=widget&limit=3",
            ctx.base(),
            ws.id.0
        ))
        .header("Authorization", auth(&mallory_tok))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hits.len(), 3, "the limit is honored with accessible hits");
    for h in &hits {
        let body = h["body"].as_str().unwrap_or("");
        assert!(
            body.contains("public"),
            "every hit is from the public channel, got {body:?}"
        );
        assert!(!body.contains("secret"), "no private-channel hit leaks");
    }

    // Alice (member of both) sees the private hits too.
    let alice_hits: Vec<Value> = ctx
        .client
        .get(format!(
            "{}/workspaces/{}/search?q=widget&limit=20",
            ctx.base(),
            ws.id.0
        ))
        .header("Authorization", auth(&alice_tok))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        alice_hits
            .iter()
            .any(|h| h["body"].as_str().unwrap_or("").contains("secret")),
        "a member still sees private-channel hits"
    );
}
