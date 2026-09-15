//! Cluster 393: REST snapshot + since-LSN catch-up. Graph is admin-gated;
//! pruned-prefix catch-up fails closed with a snapshot href; a tampered
//! chain is 409 event-log-broken.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{
    capability::{TOKEN_ADMIN, WORKSPACE_READ},
    hash_secret, TokenSecret,
};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    verify_snapshot, CatchUpPage, LogSnapshot, MemberKind, NewApiToken, NewMember, NewWorkspace,
    CATCH_UP_TYPE, LOG_SNAPSHOT_TYPE,
};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    pool: sqlx::SqlitePool,
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
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
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
    Harness {
        addr,
        server,
        client: reqwest::Client::new(),
        store,
        pool,
        _dir: dir,
    }
}

async fn mint(
    store: &dyn Store,
    ws: maidan_types::WorkspaceId,
    member: maidan_types::MemberId,
    caps: Vec<String>,
) -> String {
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

#[tokio::test]
async fn snapshot_then_catch_up_then_tamper_and_prune_fail_closed() {
    let h = spawn().await;
    let (ws, _) = h
        .store
        .create_workspace_with_event(NewWorkspace {
            name: "snap".into(),
        })
        .await
        .unwrap();
    let (member, stored) = h
        .store
        .create_member_with_event(NewMember {
            workspace_id: ws.id,
            handle: "reader".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let reader = mint(
        h.store.as_ref(),
        ws.id,
        member.id,
        vec![WORKSPACE_READ.into()],
    )
    .await;
    let admin = mint(
        h.store.as_ref(),
        ws.id,
        member.id,
        vec![WORKSPACE_READ.into(), TOKEN_ADMIN.into()],
    )
    .await;

    let header = h
        .client
        .get(format!("{}/workspaces/{}/snapshot", h.base(), ws.id.0))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_eq!(header.status(), StatusCode::OK);
    let header: serde_json::Value = header.json().await.unwrap();
    assert_eq!(header["$type"], LOG_SNAPSHOT_TYPE);
    assert!(header.get("graph").is_none());
    assert!(header["graph_hash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    let as_of = header["as_of_lsn"].as_i64().unwrap();
    assert!(as_of > 0);

    let denied = h
        .client
        .get(format!(
            "{}/workspaces/{}/snapshot?include_graph=true",
            h.base(),
            ws.id.0
        ))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let full = h
        .client
        .get(format!(
            "{}/workspaces/{}/snapshot?include_graph=true",
            h.base(),
            ws.id.0
        ))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_eq!(full.status(), StatusCode::OK);
    let snap: LogSnapshot = full.json().await.unwrap();
    assert!(snap.graph.is_some());
    assert!(verify_snapshot(&snap).ok);

    let page = h
        .client
        .get(format!(
            "{}/workspaces/{}/events/catch-up?after_lsn=0",
            h.base(),
            ws.id.0
        ))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let page: CatchUpPage = page.json().await.unwrap();
    assert_eq!(page.type_id, CATCH_UP_TYPE);
    assert!(page.ok());
    assert!(page.events.len() >= 2);

    let at_head = h
        .client
        .get(format!(
            "{}/workspaces/{}/events/catch-up?after_lsn={as_of}",
            h.base(),
            ws.id.0
        ))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_eq!(at_head.status(), StatusCode::OK);
    let at_head: CatchUpPage = at_head.json().await.unwrap();
    assert!(at_head.events.is_empty());
    assert!(at_head.ok());

    let mut payload = stored.payload.clone();
    payload["kind"] = serde_json::json!("message_posted");
    sqlx::query("UPDATE maidan_events SET payload = ? WHERE id = ?")
        .bind(payload.to_string())
        .bind(stored.id)
        .execute(&h.pool)
        .await
        .unwrap();

    let broken = h
        .client
        .get(format!(
            "{}/workspaces/{}/events/catch-up?after_lsn=0",
            h.base(),
            ws.id.0
        ))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_eq!(broken.status(), StatusCode::CONFLICT);
    let problem: serde_json::Value = broken.json().await.unwrap();
    assert_eq!(
        problem["type"],
        "https://maidan.dev/problems/event-log-broken"
    );
    assert!(problem["detail"]
        .as_str()
        .unwrap()
        .contains("content_hash_mismatch"));

    h.shutdown().await;
}

#[tokio::test]
async fn catch_up_pruned_prefix_is_409_with_snapshot_href() {
    let h = spawn().await;
    let (ws, _) = h
        .store
        .create_workspace_with_event(NewWorkspace {
            name: "prune".into(),
        })
        .await
        .unwrap();
    let (member, _) = h
        .store
        .create_member_with_event(NewMember {
            workspace_id: ws.id,
            handle: "reader".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    h.store
        .append_event(&maidan_types::Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .unwrap();
    let token = mint(
        h.store.as_ref(),
        ws.id,
        member.id,
        vec![WORKSPACE_READ.into()],
    )
    .await;
    let events = h.store.list_events_after(ws.id, 0, 50).await.unwrap();
    assert!(events.len() >= 3);
    let first = events[0].id;
    let floor = events[1].id;
    let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
    h.store.prune_events(cutoff, floor, 10).await.unwrap();

    let resp = h
        .client
        .get(format!(
            "{}/workspaces/{}/events/catch-up?after_lsn={first}",
            h.base(),
            ws.id.0
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["type"], "https://maidan.dev/problems/cursor-too-old");
    assert_eq!(body["must_refetch"], true);
    assert_eq!(body["snapshot"], LogSnapshot::path(ws.id));

    h.shutdown().await;
}
