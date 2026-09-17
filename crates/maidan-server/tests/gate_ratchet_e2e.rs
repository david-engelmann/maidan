//! A governance gate ratchets.
//!
//! Both close-gates were guarded by `thread:transition` on *both* sides — the
//! capability a close already needs, and one `maidan.agent.worker` carries. So
//! the constrained agent could delete its own constraint and then close, in two
//! calls, leaving no audit row and no event. These tests pin the asymmetry:
//! tightening stays `thread:transition`, loosening needs `channel:admin`.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

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

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
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
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

/// A worker token (the `maidan.agent.worker` capability bundle) and an admin
/// token, plus a thread to gate.
struct Room {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    worker: String,
    admin: String,
    thread: uuid::Uuid,
    reviewer: MemberId,
}

async fn room() -> Room {
    let (addr, client, store) = spawn().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let mk = |handle: &'static str| {
        let store = store.clone();
        let ws = ws.id;
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
        }
    };
    let agent = mk("agent").await;
    let boss = mk("boss").await;
    let reviewer = mk("reviewer").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .unwrap();
    // Exactly the worker bundle: transition, but no channel:admin.
    let worker = mint(
        store.as_ref(),
        ws.id,
        agent.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
            capability::THREAD_TRANSITION.into(),
        ],
    )
    .await;
    let admin = mint(
        store.as_ref(),
        ws.id,
        boss.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::THREAD_TRANSITION.into(),
            capability::CHANNEL_ADMIN.into(),
        ],
    )
    .await;
    Room {
        base: format!("http://{addr}"),
        client,
        store,
        worker,
        admin,
        thread: thread.id.0,
        reviewer: reviewer.id,
    }
}

impl Room {
    async fn send(
        &self,
        method: &str,
        path: &str,
        tok: &str,
        body: Option<serde_json::Value>,
    ) -> StatusCode {
        let url = format!("{}{path}", self.base);
        let mut req = match method {
            "PUT" => self.client.put(url),
            "POST" => self.client.post(url),
            "DELETE" => self.client.delete(url),
            _ => unreachable!(),
        }
        .header("authorization", format!("Bearer {tok}"));
        if let Some(b) = body {
            req = req.json(&b);
        }
        req.send().await.unwrap().status()
    }
}

/// The headline. A worker arms the land gate, then cannot remove it; an admin
/// can. Before 397.2 the first DELETE returned 204 and the gate was gone.
#[tokio::test]
async fn a_worker_can_arm_a_land_gate_but_not_clear_it() {
    let r = room().await;
    let req_path = format!("/threads/{}/land-gate/requirement", r.thread);
    let gate_path = format!("/threads/{}/land-gate", r.thread);

    // Tightening: the worker's own capability is enough.
    assert_eq!(
        r.send("PUT", &req_path, &r.worker, None).await,
        StatusCode::OK,
        "arming stays thread:transition"
    );
    // Loosening: refused.
    assert_eq!(
        r.send("DELETE", &gate_path, &r.worker, None).await,
        StatusCode::FORBIDDEN,
        "a worker must not be able to delete the gate constraining it"
    );
    // The gate is still armed.
    let standing = r
        .store
        .get_land_gate_standing(maidan_types::ThreadId(r.thread))
        .await
        .unwrap();
    assert!(standing.required, "the gate survived the refused clear");

    // An admin may waive it, and that waiver is on the audit trail.
    assert_eq!(
        r.send("DELETE", &gate_path, &r.admin, None).await,
        StatusCode::NO_CONTENT
    );
    let audit = r.store.list_audit(200).await.unwrap();
    assert!(
        audit.iter().any(|a| a.action == "land_gate.clear"),
        "a waiver must leave a trace; got {:?}",
        audit.iter().map(|a| &a.action).collect::<Vec<_>>()
    );
}

/// The review requirement ratchets on value, not just existence: raising `k` is
/// a tightening, lowering it (including to 0, which disarms) is the waiver.
#[tokio::test]
async fn a_worker_can_raise_k_but_not_lower_it() {
    let r = room().await;
    let path = format!("/threads/{}/review-requirement", r.thread);

    assert_eq!(
        r.send("PUT", &path, &r.worker, Some(json!({"required_count": 2})))
            .await,
        StatusCode::OK,
        "raising k stays thread:transition"
    );
    assert_eq!(
        r.send("PUT", &path, &r.worker, Some(json!({"required_count": 0})))
            .await,
        StatusCode::FORBIDDEN,
        "zeroing k disarms the gate, so it is a waiver"
    );
    assert_eq!(
        r.send("PUT", &path, &r.worker, Some(json!({"required_count": 1})))
            .await,
        StatusCode::FORBIDDEN,
        "any lowering is a waiver"
    );
    assert_eq!(
        r.send("DELETE", &path, &r.worker, None).await,
        StatusCode::FORBIDDEN,
        "deleting the requirement is a waiver"
    );

    // Still 2 — none of the refused calls took effect.
    let req = r
        .store
        .get_review_requirement(maidan_types::ThreadId(r.thread))
        .await
        .unwrap()
        .expect("requirement exists");
    assert_eq!(req.required_count, 2);

    // The admin can lower it, audited.
    assert_eq!(
        r.send("PUT", &path, &r.admin, Some(json!({"required_count": 1})))
            .await,
        StatusCode::OK
    );
    let audit = r.store.list_audit(200).await.unwrap();
    assert!(audit.iter().any(|a| a.action == "review_requirement.lower"));
}

/// Un-designating a reviewer widens who may approve — an empty named set means
/// *any* non-implementer approval counts — so it is a loosening too.
#[tokio::test]
async fn a_worker_cannot_undesignate_a_reviewer() {
    let r = room().await;
    let list = format!("/threads/{}/reviewers", r.thread);
    let one = format!("/threads/{}/reviewers/{}", r.thread, r.reviewer.0);

    assert_eq!(
        r.send(
            "POST",
            &list,
            &r.worker,
            Some(json!({"member_id": r.reviewer.0}))
        )
        .await,
        StatusCode::NO_CONTENT,
        "naming a reviewer is a tightening"
    );
    assert_eq!(
        r.send("DELETE", &one, &r.worker, None).await,
        StatusCode::FORBIDDEN,
        "emptying the eligible set widens the gate"
    );
    assert_eq!(
        r.send("DELETE", &one, &r.admin, None).await,
        StatusCode::NO_CONTENT
    );
}
