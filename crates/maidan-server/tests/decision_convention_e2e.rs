//! Cluster 325: the agent conventions (decision records, supersession, grounding
//! acks) are expressible over the EXISTING API with no new server object — this
//! proves the "room supports the pattern". Auth ENABLED (real token): thread
//! results/votes persist `produced_by`/actor as NOT-NULL FKs.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EditMessage, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread,
    NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

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
                capability::THREAD_TRANSITION.into(),
            ],
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
        false, // auth ENABLED
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

#[tokio::test]
async fn decision_supersession_and_ack_conventions_work_over_the_api() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "d".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "architect".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "decisions".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let make_thread = |title: &str| {
        let store = store.clone();
        let cid = channel.id;
        let title = title.to_string();
        async move {
            store
                .create_thread(NewThread {
                    channel_id: cid,
                    parent_thread_id: None,
                    title: Some(title),
                })
                .await
                .unwrap()
        }
    };
    let decision_a = make_thread("use postgres").await;
    let decision_b = make_thread("use postgres + pgvector").await;
    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let bearer = format!("Bearer {tok}");

    // --- decision record on A (thread_result JSON convention) ---
    let put = client
        .put(format!("{base}/threads/{}/result", decision_a.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "result": {
            "kind": "decision",
            "status": "accepted",
            "context": "need a database",
            "decision": "postgres",
            "consequences": "sql everywhere",
            "alternatives": ["sqlite only"]
        }}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let got: Value = client
        .get(format!("{base}/threads/{}/result", decision_a.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["result"]["kind"], json!("decision"));
    assert_eq!(got["result"]["status"], json!("accepted"));

    // --- supersession: B supersedes A (typed reference edge) ---
    let refr = client
        .post(format!("{base}/references"))
        .header("Authorization", &bearer)
        .json(&json!({
            "src_kind": "thread", "src_id": decision_b.id.0,
            "dst_kind": "thread", "dst_id": decision_a.id.0,
            "relation": "supersedes"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(refr.status(), StatusCode::CREATED);

    // Flip A's status to superseded (re-set the record).
    let _ = client
        .put(format!("{base}/threads/{}/result", decision_a.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "result": {
            "kind": "decision", "status": "superseded",
            "decision": "postgres", "superseded_by": decision_b.id.0
        }}))
        .send()
        .await
        .unwrap();
    let got: Value = client
        .get(format!("{base}/threads/{}/result", decision_a.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["result"]["status"], json!("superseded"));

    // "What replaced A?" — the reverse, relation-filtered query.
    let supersedes: Value = client
        .get(format!(
            "{base}/references?dst_kind=thread&dst_id={}&relation=supersedes",
            decision_a.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let arr = supersedes.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["src_id"], json!(decision_b.id.0));

    // --- grounding ack, version-pinned by time ---
    let msg = store
        .post_message(NewMessage {
            thread_id: decision_a.id,
            author_id: member.id,
            body: "final spec v1".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();
    let ack = client
        .post(format!("{base}/messages/{}/votes", msg.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "member_id": member.id.0, "kind": "ack", "confidence": 0.9 }))
        .send()
        .await
        .unwrap();
    assert_eq!(ack.status(), StatusCode::NO_CONTENT);
    let votes: Value = client
        .get(format!("{base}/messages/{}/votes", msg.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ack_vote = votes
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == json!("ack"))
        .expect("ack vote present");
    let ack_at = ack_vote["created_at"].as_str().unwrap().to_string();

    // Edit the message AFTER the ack — a 1.1s gap so the edit's timestamp is
    // strictly later even at SQLite's second-granularity `datetime('now')`.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    store
        .edit_message(
            msg.id,
            member.id,
            EditMessage {
                body: "final spec v2".into(),
                metadata: json!({}),
                content: None,
            },
        )
        .await
        .unwrap();

    // The ack is now detectably stale: it grounded the message as of ack_at, but
    // the message was edited after that. Both timestamps come from the API.
    let ctx: Value = client
        .get(format!("{base}/threads/{}/context", decision_a.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let edits = ctx["message_edits"].as_array().unwrap();
    let last_edit_at = edits
        .iter()
        .filter(|e| e["message_id"] == json!(msg.id.0))
        .filter_map(|e| e["edited_at"].as_str())
        .max()
        .expect("an edit is recorded");
    assert!(
        last_edit_at > ack_at.as_str(),
        "edit ({last_edit_at}) after ack ({ack_at}) => ack is stale"
    );
}
