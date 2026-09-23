//! Every durable record of an action names who acted, on whose behalf, and under
//! which grant. A member acting for itself is recorded as both actor and
//! subject; a delegate acting for someone is recorded as the actor, with the
//! member as subject and the grant it used — and because that record lives in
//! the hashed event payload, changing it breaks the chain.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use chrono::{Duration as ChronoDuration, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Attribution, EventKind, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread,
    NewWorkspace, StoredEvent, ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct World {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    pool: sqlx::SqlitePool,
    ws: WorkspaceId,
    thread: ThreadId,
    orchestrator: MemberId,
    worker: MemberId,
    admin_tok: String,
    orch_tok: String,
    worker_tok: String,
    _dir: tempfile::TempDir,
}

async fn world() -> World {
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for handle in ["admin", "orchestrator", "worker"] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
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
            title: Some("t".into()),
        })
        .await
        .unwrap();
    let mint = |member_id: MemberId, caps: Vec<&str>| {
        let store = store.clone();
        let caps: Vec<String> = caps.into_iter().map(String::from).collect();
        async move {
            let secret = TokenSecret::generate();
            store
                .create_api_token(NewApiToken {
                    workspace_id: ws.id,
                    member_id,
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
    };
    let work = vec![capability::WORKSPACE_READ, capability::MESSAGE_POST];
    let admin_tok = mint(ids[0], vec![capability::TOKEN_ADMIN]).await;
    let orch_tok = mint(ids[1], work.clone()).await;
    let worker_tok = mint(ids[2], work).await;

    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });

    World {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        store,
        pool,
        ws: ws.id,
        thread: thread.id,
        orchestrator: ids[1],
        worker: ids[2],
        admin_tok,
        orch_tok,
        worker_tok,
        _dir: dir,
    }
}

impl World {
    /// A token borrowed by the orchestrator to act as the worker, and its grant.
    async fn borrowed(&self) -> (String, uuid::Uuid) {
        let grant: Value = self
            .client
            .post(format!(
                "{}/workspaces/{}/delegation-grants",
                self.base, self.ws.0
            ))
            .bearer_auth(&self.admin_tok)
            .json(&json!({
                "subject_id": self.worker.0,
                "delegate_id": self.orchestrator.0,
                "capabilities": [capability::WORKSPACE_READ, capability::MESSAGE_POST],
                "purpose": "post for the worker",
                "expires_at": Utc::now() + ChronoDuration::hours(1),
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let exchanged: Value = self
            .client
            .post(format!("{}/tokens/delegate", self.base))
            .bearer_auth(&self.orch_tok)
            .json(&json!({ "grant_id": grant["id"] }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        (
            exchanged["token"]["secret"].as_str().unwrap().to_string(),
            grant["id"].as_str().unwrap().parse().unwrap(),
        )
    }

    async fn post(&self, bearer: &str, body: &str) -> Value {
        let resp = self
            .client
            .post(format!("{}/threads/{}/messages", self.base, self.thread.0))
            .bearer_auth(bearer)
            .json(&json!({ "body": body }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "post {body}");
        resp.json().await.unwrap()
    }

    /// The `MessagePosted` event for the message with `body`.
    async fn posted_event(&self, body: &str) -> StoredEvent {
        self.store
            .list_events_after(self.ws, 0, 500)
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.kind == EventKind::MessagePosted && e.payload["message"]["body"] == body)
            .unwrap_or_else(|| panic!("no MessagePosted event for {body}"))
    }
}

#[tokio::test]
async fn a_direct_action_names_its_author_as_both_actor_and_subject() {
    let w = world().await;
    w.post(&w.worker_tok, "direct").await;
    assert_eq!(
        w.posted_event("direct").await.attribution(),
        Some(Attribution {
            actor_id: w.worker,
            subject_id: w.worker,
            grant_id: None,
        })
    );
}

#[tokio::test]
async fn a_delegated_action_names_the_delegate_as_actor() {
    let w = world().await;
    let (borrowed, grant_id) = w.borrowed().await;
    let message = w.post(&borrowed, "on behalf").await;

    // The message is the worker's: it is shown as theirs and counts as theirs.
    assert_eq!(message["author_id"], w.worker.0.to_string());
    // But the record says who really wrote it.
    let attribution = w.posted_event("on behalf").await.attribution().unwrap();
    assert_eq!(attribution.actor_id, w.orchestrator, "the delegate acted");
    assert_eq!(attribution.subject_id, w.worker, "for the worker");
    assert_eq!(attribution.grant_id.map(|g| g.0), Some(grant_id));
    assert!(attribution.is_delegated());
}

#[tokio::test]
async fn a_delegated_audit_row_names_actor_subject_and_grant() {
    let w = world().await;
    let (borrowed, grant_id) = w.borrowed().await;
    let narrowed = w
        .client
        .post(format!("{}/tokens/attenuate", w.base))
        .bearer_auth(&borrowed)
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ] }))
        .send()
        .await
        .unwrap();
    assert_eq!(narrowed.status(), StatusCode::CREATED);

    let mint = w
        .store
        .list_audit_for_workspace(w.ws, 50)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.action == "token.mint")
        .expect("narrowing writes a token.mint audit row");
    assert_eq!(mint.actor_id, Some(w.orchestrator), "the delegate acted");
    assert_eq!(mint.subject_id, Some(w.worker), "for the worker");
    assert_eq!(mint.grant_id.map(|g| g.0), Some(grant_id));
}

/// Attribution is stored inside the hashed payload, so rewriting who did
/// something — here, erasing that a delegate did it — is a chain break.
#[tokio::test]
async fn changing_who_did_something_breaks_the_chain() {
    let w = world().await;
    let (borrowed, _) = w.borrowed().await;
    w.post(&borrowed, "tamper target").await;
    assert!(w.store.verify_event_chain(w.ws).await.unwrap().ok);

    let event = w.posted_event("tamper target").await;
    let mut forged = event.payload.clone();
    forged["attribution"] = json!({
        "actor_id": w.worker.0,
        "subject_id": w.worker.0,
    });
    sqlx::query("UPDATE maidan_events SET payload = ? WHERE id = ?")
        .bind(serde_json::to_string(&forged).unwrap())
        .bind(event.id)
        .execute(&w.pool)
        .await
        .unwrap();

    assert!(
        !w.store.verify_event_chain(w.ws).await.unwrap().ok,
        "hiding that a delegate acted must break the chain"
    );
}
