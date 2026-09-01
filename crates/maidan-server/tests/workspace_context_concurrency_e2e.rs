//! Cluster 199 (Arc D): the workspace-context pack builds each thread's context
//! concurrently (bounded). This guards the correctness invariants that
//! parallelization must not break: every page thread is built, each context
//! carries *its own* messages (no cross-contamination), and the output stays in
//! page order.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
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

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
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
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
            ],
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
        false,
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

fn bearer(t: &str) -> String {
    format!("Bearer {t}")
}

#[tokio::test]
async fn workspace_context_builds_each_thread_with_its_own_messages() {
    let ctx = spawn().await;
    let base = ctx.base();
    let ws = ctx
        .store
        .create_workspace(NewWorkspace { name: "ctx".into() })
        .await
        .unwrap();
    let alice = ctx
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let tok = mint(ctx.store.as_ref(), ws.id, alice.id).await;

    let ch: Value = ctx
        .client
        .post(format!("{base}/workspaces/{}/channels", ws.id.0))
        .header("Authorization", bearer(&tok))
        .json(&json!({"name": "work"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();

    // Several threads, each with a message uniquely tied to its thread id — the
    // signal for cross-contamination under concurrent assembly.
    const N: usize = 12;
    let mut expected: Vec<(String, String)> = Vec::new(); // (thread_id, body)
    for i in 0..N {
        let th: Value = ctx
            .client
            .post(format!("{base}/channels/{cid}/threads"))
            .header("Authorization", bearer(&tok))
            .json(&json!({"title": format!("t{i}")}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let tid = th["id"].as_str().unwrap().to_string();
        let body = format!("body-for-{tid}");
        ctx.client
            .post(format!("{base}/threads/{tid}/messages"))
            .header("Authorization", bearer(&tok))
            .json(&json!({"author_id": alice.id.0, "body": body}))
            .send()
            .await
            .unwrap();
        expected.push((tid, body));
    }

    let pack: Value = ctx
        .client
        .get(format!(
            "{base}/workspaces/{}/context?thread_limit=50",
            ws.id.0
        ))
        .header("Authorization", bearer(&tok))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let threads = pack["threads"].as_array().unwrap();
    assert_eq!(threads.len(), N, "every page thread is built");

    // Each built context carries exactly its own message (no cross-thread mixups
    // from the concurrent build), and the threads are in page order.
    for (i, tc) in threads.iter().enumerate() {
        let tid = tc["thread"]["id"].as_str().unwrap();
        let msgs = tc["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1, "thread {tid} has its single message");
        assert_eq!(
            msgs[0]["body"].as_str().unwrap(),
            format!("body-for-{tid}"),
            "thread {tid} context carries its OWN message"
        );
        assert_eq!(
            tid, expected[i].0,
            "threads returned in page (creation) order"
        );
    }
}
