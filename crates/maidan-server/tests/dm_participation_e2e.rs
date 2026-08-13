//! Cluster 203: a non-participant cannot tail a DM's live events by supplying
//! its `dm_conversation_id` or its `thread_id` on `GET /mcp/stream`. The gate is
//! `expand_event_filter` → `ensure_thread_access` (DM-participant-aware, Cluster
//! 180). A participant still subscribes fine.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
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

async fn mint_subscriber(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::EVENT_SUBSCRIBE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn non_participant_cannot_tail_a_dm() {
    let ctx = spawn().await;
    let base = ctx.base();
    let ws = ctx
        .store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let alice = mk_member(ctx.store.as_ref(), ws.id, "alice").await;
    let bob = mk_member(ctx.store.as_ref(), ws.id, "bob").await;
    let carol = mk_member(ctx.store.as_ref(), ws.id, "carol").await; // outsider

    let dm = ctx
        .store
        .open_dm_conversation(ws.id, alice, bob)
        .await
        .unwrap();

    let carol_tok = mint_subscriber(ctx.store.as_ref(), ws.id, carol).await;
    let alice_tok = mint_subscriber(ctx.store.as_ref(), ws.id, alice).await;

    // Carol (not a participant) cannot tail the DM by its dm_conversation_id...
    let via_dm = ctx
        .client
        .get(format!(
            "{base}/mcp/stream?workspace_id={}&dm_conversation_id={}",
            ws.id.0, dm.id.0
        ))
        .header("Authorization", format!("Bearer {carol_tok}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        via_dm.status(),
        StatusCode::FORBIDDEN,
        "non-participant denied via dm_conversation_id"
    );

    // ...nor by supplying the DM's thread_id directly.
    let via_thread = ctx
        .client
        .get(format!(
            "{base}/mcp/stream?workspace_id={}&thread_id={}",
            ws.id.0, dm.thread_id.0
        ))
        .header("Authorization", format!("Bearer {carol_tok}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        via_thread.status(),
        StatusCode::FORBIDDEN,
        "non-participant denied via thread_id"
    );

    // A participant (alice) still subscribes fine.
    let ok = ctx
        .client
        .get(format!(
            "{base}/mcp/stream?workspace_id={}&dm_conversation_id={}",
            ws.id.0, dm.id.0
        ))
        .header("Authorization", format!("Bearer {alice_tok}"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK, "a participant may tail the DM");
}
