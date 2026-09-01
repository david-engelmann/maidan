//! Task structured-results REST (Cluster 235): set/get a thread's result over
//! HTTP, and the `ThreadResultSet` event on set. Auth ENABLED (real token) so
//! `produced_by` is a real member.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Event, EventFilter, EventKind, MemberId, MemberKind, NewApiToken, NewChannel, NewMember,
    NewThread, NewWorkspace, WorkspaceId,
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
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn set_get_result_over_http_and_event() {
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
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let mut subscriber = bus
        .subscribe(EventFilter::all().with_kinds([EventKind::ThreadResultSet]))
        .await
        .unwrap();

    let ws = store
        .create_workspace(NewWorkspace { name: "r".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "q".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;

    let state = AppState::new(
        store.clone(),
        artifacts,
        bus.clone(),
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
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let bearer = format!("Bearer {tok}");

    // No result yet -> 404.
    let miss = client
        .get(format!("{base}/threads/{}/result", thread.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(miss.status(), StatusCode::NOT_FOUND);

    // Set a structured result.
    let payload = json!({ "status": "done", "score": 7 });
    let set = client
        .put(format!("{base}/threads/{}/result", thread.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "result": payload }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);
    let set_body: Value = set.json().await.unwrap();
    assert_eq!(set_body["result"], payload);
    assert_eq!(set_body["produced_by"], json!(member.id.0));

    // The set published a ThreadResultSet event.
    let event = tokio::time::timeout(Duration::from_secs(2), subscriber.next())
        .await
        .expect("timeout waiting for ThreadResultSet")
        .expect("subscriber ended");
    let BusItem::Event(envelope) = event else {
        panic!("expected event, got lag/end");
    };
    match envelope.event {
        Event::ThreadResultSet {
            thread_id,
            produced_by,
            ..
        } => {
            assert_eq!(thread_id, thread.id);
            assert_eq!(produced_by, member.id);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    // GET now returns it.
    let got: Value = client
        .get(format!("{base}/threads/{}/result", thread.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["result"], payload);
}
