//! Task structured-results REST: set/get a thread's result over HTTP, and the
//! `ThreadResultSet` event on set. Auth ENABLED (real token) so `produced_by`
//! is a real member.

use maidan_store::test_support::SetResultClaiming;
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
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
            description: None,
            created_by: None,
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
    let lease = store.claim_lease_for(thread.id, member.id).await.unwrap();
    let set = client
        .put(format!("{base}/threads/{}/result", thread.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "result": payload, "claim_lease_id": lease.0 }))
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

/// A served instance over a fresh SQLite store, auth enabled.
async fn serve(store: Arc<dyn Store>, pool: sqlx::SqlitePool) -> String {
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.keep()));
    let state = AppState::new(
        store,
        artifacts,
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    format!("http://{addr}")
}

async fn fresh() -> (Arc<dyn Store>, sqlx::SqlitePool) {
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
    (Arc::new(SqliteStore::for_tests(pool.clone())), pool)
}

/// A workspace with `seats` agent members (and their tokens) and one task.
async fn team(
    store: &dyn Store,
    name: &str,
    seats: usize,
) -> (
    Vec<(MemberId, String)>,
    maidan_types::ChannelId,
    maidan_types::ThreadId,
) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let mut out = Vec::new();
    for n in 0..seats {
        let m = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: format!("seat-{n}"),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        out.push((m.id, format!("Bearer {}", mint(store, ws.id, m.id).await)));
    }
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
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    (out, channel.id, thread.id)
}

async fn put_result(
    base: &str,
    bearer: &str,
    thread: maidan_types::ThreadId,
    body: Value,
) -> (StatusCode, Value) {
    let resp = reqwest::Client::new()
        .put(format!("{base}/threads/{}/result", thread.0))
        .header("Authorization", bearer)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_seat_whose_claim_lapsed_and_was_taken_over_gets_claim_lost_and_the_new_holders_result_lands(
) {
    let (store, pool) = fresh().await;
    let (seats, channel, thread) = team(store.as_ref(), "ramp", 2).await;
    let base = serve(store.clone(), pool).await;
    let ((a, a_h), (b, b_h)) = (seats[0].clone(), seats[1].clone());
    let lease_a = store
        .claim_next_thread(channel, a, Some(1), None)
        .await
        .unwrap()
        .unwrap()
        .claim_lease_id
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let lease_b = store
        .claim_next_thread(channel, b, Some(300), None)
        .await
        .unwrap()
        .unwrap()
        .claim_lease_id
        .unwrap();

    let (status, problem) = put_result(
        &base,
        &a_h,
        thread,
        json!({ "result": { "from": "a" }, "claim_lease_id": lease_a.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert_eq!(
        problem["type"], "https://maidan.dev/problems/claim-lost",
        "{problem}"
    );
    assert!(store.get_thread_result(thread).await.unwrap().is_none());

    let (status, landed) = put_result(
        &base,
        &b_h,
        thread,
        json!({ "result": { "from": "b" }, "claim_lease_id": lease_b.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{landed}");
    assert_eq!(landed["produced_by"], json!(b.0));
}

#[tokio::test]
async fn a_result_write_without_a_claim_lease_is_refused_and_stores_nothing() {
    let (store, pool) = fresh().await;
    let (seats, _, thread) = team(store.as_ref(), "bare", 1).await;
    let base = serve(store.clone(), pool).await;
    store.claim_lease_for(thread, seats[0].0).await.unwrap();
    let (status, body) = put_result(
        &base,
        &seats[0].1,
        thread,
        json!({ "result": { "ok": true } }),
    )
    .await;
    assert!(status.is_client_error(), "{status}: {body}");
    assert!(store.get_thread_result(thread).await.unwrap().is_none());
}

#[tokio::test]
async fn a_seat_cannot_write_another_workspaces_result_with_any_lease() {
    let (store, pool) = fresh().await;
    let (one, _, thread_one) = team(store.as_ref(), "one", 1).await;
    let (two, _, thread_two) = team(store.as_ref(), "two", 1).await;
    let base = serve(store.clone(), pool).await;
    let lease_one = store.claim_lease_for(thread_one, one[0].0).await.unwrap();
    let lease_two = store.claim_lease_for(thread_two, two[0].0).await.unwrap();

    for lease in [lease_one, lease_two] {
        let (status, body) = put_result(
            &base,
            &two[0].1,
            thread_one,
            json!({ "result": { "from": "two" }, "claim_lease_id": lease.0 }),
        )
        .await;
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
            "workspace two reaches one's thread: {status} {body}"
        );
    }
    assert!(store.get_thread_result(thread_one).await.unwrap().is_none());
    let (status, _) = put_result(
        &base,
        &one[0].1,
        thread_one,
        json!({ "result": { "from": "one" }, "claim_lease_id": lease_one.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
