//! Explicit dispatch-block REST (Cluster 386, Wave 2 #27, G14 + W2): set/get/
//! list, claim_next skip, explicit-claim 409, and `BlockedResolved` on DELETE.
//! Distinct from Cluster 363 unclaimable and Cluster 218 DAG readiness.
//! Auth ENABLED (real token) so `set_by` / `resolved_by` are real members.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    BlockedReason, Event, EventFilter, EventKind, MemberKind, NewApiToken, NewChannel, NewMember,
    NewThread, NewWorkspace,
};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn blocked_reason_parks_a_thread_from_dispatch_and_emits_on_clear() {
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

    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let mut subscriber = bus
        .subscribe(EventFilter::all().with_kinds([EventKind::BlockedResolved]))
        .await
        .unwrap();
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
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let mk = |title: &str| {
        let store = store.clone();
        let cid = ch.id;
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
    let t1 = mk("t1").await;
    let t2 = mk("t2").await;

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: agent.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("b".into()),
            capabilities: vec!["workspace:read".into(), "thread:transition".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());

    // Set a closed reason on t1.
    let put = client
        .put(format!("{base}/threads/{}/block", t1.id.0))
        .header("Authorization", &auth)
        .json(&json!({ "reason": "gate" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let put_body: serde_json::Value = put.json().await.unwrap();
    assert_eq!(put_body["reason"], "gate");
    assert_eq!(put_body["set_by"], agent.id.0.to_string());

    // GET the block.
    let got: serde_json::Value = client
        .get(format!("{base}/threads/{}/block", t1.id.0))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["reason"], "gate");

    // Unknown reason is a 400 (closed enum).
    let bad = client
        .put(format!("{base}/threads/{}/block", t2.id.0))
        .header("Authorization", &auth)
        .json(&json!({ "reason": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    // Channel list shows the one blocked thread.
    let listed: serde_json::Value = client
        .get(format!("{base}/channels/{}/blocked", ch.id.0))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let arr = listed.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["thread_id"], t1.id.0.to_string());
    assert_eq!(arr[0]["reason"], "gate");

    let claim = |thread: uuid::Uuid| {
        let (client, base, auth) = (client.clone(), base.clone(), auth.clone());
        async move {
            client
                .post(format!("{base}/threads/{thread}/assignee/claim"))
                .header("Authorization", &auth)
                .json(&json!({ "member_id": agent.id.0 }))
                .send()
                .await
                .unwrap()
        }
    };

    // Explicit claim of the blocked thread is refused (409).
    assert_eq!(claim(t1.id.0).await.status(), StatusCode::CONFLICT);

    // claim_next skips the older blocked t1 and claims t2.
    let next: serde_json::Value = client
        .post(format!("{base}/channels/{}/threads/claim-next", ch.id.0))
        .header("Authorization", &auth)
        .json(&json!({ "member_id": agent.id.0 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(next["id"], t2.id.0.to_string(), "the blocked t1 is skipped");

    // Unblock: 204 the first time; bus emits BlockedResolved; 404 the second.
    assert_eq!(
        client
            .delete(format!("{base}/threads/{}/block", t1.id.0))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let event = tokio::time::timeout(Duration::from_secs(2), subscriber.next())
        .await
        .expect("timeout waiting for BlockedResolved")
        .expect("subscriber ended");
    let BusItem::Event(envelope) = event else {
        panic!("expected event, got lag/end");
    };
    match envelope.event {
        Event::BlockedResolved {
            thread_id,
            reason,
            resolved_by,
            ..
        } => {
            assert_eq!(thread_id, t1.id);
            assert_eq!(reason, BlockedReason::Gate);
            assert_eq!(resolved_by, agent.id);
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert_eq!(
        client
            .delete(format!("{base}/threads/{}/block", t1.id.0))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .get(format!("{base}/threads/{}/block", t1.id.0))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    // Now t1 is claimable.
    assert_eq!(claim(t1.id.0).await.status(), StatusCode::OK);

    server.abort();
}
