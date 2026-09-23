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
    let admin_tok = mint(
        ids[0],
        vec![
            capability::TOKEN_ADMIN,
            capability::WORKSPACE_READ,
            capability::EVENT_SUBSCRIBE,
        ],
    )
    .await;
    let orch_tok = mint(ids[1], work.clone()).await;
    let worker_tok = mint(ids[2], work).await;

    let mut state = AppState::new(
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
    state.subscribe_resume_secret = Some(Arc::from(
        maidan_server::subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET,
    ));
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
    assert!(
        w.mutations().await.is_empty(),
        "a change that wrote its own audit row gets no second record"
    );
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

impl World {
    /// Audit rows written for changes that did not record themselves.
    async fn mutations(&self) -> Vec<maidan_types::AuditEvent> {
        self.store
            .list_audit(500)
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.action == "mutation")
            .collect()
    }

    async fn set_email(&self, bearer: &str, member: MemberId, email: &str) -> StatusCode {
        self.client
            .put(format!("{}/members/{}/email", self.base, member.0))
            .bearer_auth(bearer)
            .json(&json!({ "email": email }))
            .send()
            .await
            .unwrap()
            .status()
    }
}

/// Setting a delivery address writes no event and no audit row of its own —
/// one of 85 such routes. The request layer records it anyway.
#[tokio::test]
async fn a_change_that_records_nothing_itself_still_leaves_a_record() {
    let w = world().await;
    assert_eq!(
        w.set_email(&w.worker_tok, w.worker, "worker@example.com")
            .await,
        StatusCode::OK
    );
    let rows = w.mutations().await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.metadata["operation"], "PUT /members/{id}/email");
    assert_eq!(
        row.metadata["path"],
        format!("/members/{}/email", w.worker.0)
    );
    assert_eq!(row.metadata["surface"], "rest");
    assert_eq!(row.actor_id, Some(w.worker));
    assert_eq!(row.subject_id, Some(w.worker));
    assert_eq!(row.grant_id, None);
}

#[tokio::test]
async fn a_delegated_change_records_the_delegate_the_member_and_the_grant() {
    let w = world().await;
    let (borrowed, grant) = w.borrowed().await;
    assert_eq!(
        w.set_email(&borrowed, w.worker, "redirected@example.com")
            .await,
        StatusCode::OK
    );
    let rows = w.mutations().await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].actor_id, Some(w.orchestrator));
    assert_eq!(rows[0].subject_id, Some(w.worker));
    assert_eq!(rows[0].grant_id.map(|g| g.0), Some(grant));
}

/// Posting appends an attributed event, so the request layer has nothing to
/// add — a second record of the same change would only be noise.
#[tokio::test]
async fn a_change_that_records_itself_is_not_recorded_twice() {
    let w = world().await;
    w.post(&w.worker_tok, "recorded by its event").await;
    assert!(w.mutations().await.is_empty());
}

#[tokio::test]
async fn reads_and_refusals_are_not_changes() {
    let w = world().await;
    let read = w
        .client
        .get(format!("{}/threads/{}", w.base, w.thread.0))
        .bearer_auth(&w.worker_tok)
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    // The worker's token carries no `thread:transition`.
    let refused = w
        .client
        .put(format!("{}/threads/{}/title", w.base, w.thread.0))
        .bearer_auth(&w.worker_tok)
        .json(&json!({ "title": "renamed" }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(w.mutations().await.is_empty());
}

#[tokio::test]
async fn an_mcp_tool_that_records_nothing_itself_still_leaves_a_record() {
    let w = world().await;
    let call = |tool: &str, args: Value| {
        w.client
            .post(format!("{}/mcp", w.base))
            .bearer_auth(&w.worker_tok)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": tool, "arguments": args }
            }))
            .send()
    };
    let read: Value = call("list_channels", json!({ "workspace_id": w.ws.0 }))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(read["error"].is_null(), "{read}");
    assert!(
        w.mutations().await.is_empty(),
        "a read tool is not a change"
    );

    let set: Value = call(
        "set_member_email",
        json!({ "member_id": w.worker.0, "email": "worker@example.com" }),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(set["error"].is_null(), "{set}");
    let rows = w.mutations().await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].metadata["surface"], "mcp");
    assert_eq!(rows[0].metadata["operation"], "tools/call:set_member_email");
    assert_eq!(rows[0].metadata["ids"]["member_id"], w.worker.0.to_string());
    assert!(
        rows[0].metadata["ids"].get("email").is_none(),
        "only ids are kept from arguments"
    );
    assert_eq!(rows[0].actor_id, Some(w.worker));
}

impl World {
    /// Subscribe to the workspace's live stream, `lean` or full frames, and
    /// return once the subscription is acknowledged.
    async fn subscribe(
        &self,
        lean: bool,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
        let url = format!("{}/ws/subscribe", self.base.replacen("http", "ws", 1));
        let (mut ws, _) = tokio_tungstenite::connect_async(url.into_client_request().unwrap())
            .await
            .unwrap();
        ws.send(Message::Text(
            json!({
                "filter": { "workspace_id": self.ws.0 },
                "token": self.admin_tok,
                "lean": lean,
            })
            .to_string(),
        ))
        .await
        .unwrap();
        loop {
            let Some(Ok(Message::Text(text))) = ws.next().await else {
                panic!("subscription closed before its ack");
            };
            let frame: Value = serde_json::from_str(&text).unwrap();
            if frame["type"] == "subscribe_ack" {
                return ws;
            }
        }
    }
}

/// The next `message_posted` frame on `ws`.
async fn next_post(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let deadline = std::time::Duration::from_secs(5);
    loop {
        let next = tokio::time::timeout(deadline, ws.next())
            .await
            .expect("no message_posted frame within 5s");
        if let Some(Ok(Message::Text(text))) = next {
            let frame: Value = serde_json::from_str(&text).unwrap();
            if frame["kind"] == "message_posted" {
                return frame;
            }
        }
    }
}

/// A live subscriber learns who acted from the frame itself, as a replay
/// would from the stored event — it does not have to refetch to tell a
/// delegated post from a direct one.
#[tokio::test]
async fn a_live_frame_names_who_acted_and_for_whom() {
    let w = world().await;
    let (borrowed, grant) = w.borrowed().await;
    for lean in [false, true] {
        let mut ws = w.subscribe(lean).await;

        w.post(&w.worker_tok, &format!("direct lean={lean}")).await;
        let direct = next_post(&mut ws).await;
        assert_eq!(
            direct["attribution"],
            json!({ "actor_id": w.worker.0, "subject_id": w.worker.0 }),
            "lean={lean}: {direct}"
        );

        w.post(&borrowed, &format!("delegated lean={lean}")).await;
        let delegated = next_post(&mut ws).await;
        assert_eq!(
            delegated["attribution"],
            json!({
                "actor_id": w.orchestrator.0,
                "subject_id": w.worker.0,
                "grant_id": grant,
            }),
            "lean={lean}: {delegated}"
        );
    }
}
