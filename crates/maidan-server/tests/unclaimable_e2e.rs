//! Unclaimable (Cluster 363, G3): park/un-park a thread over REST, the channel
//! list, claim_next skip, and the explicit-claim 409.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn unclaimable_parks_a_thread_from_dispatch() {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    // Auth ENABLED: the park handler persists `auth.member_id` (the `marked_by` FK),
    // which the for_tests bypass (nil member) would FK-fail — so mint a real token.
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
        .create_workspace(NewWorkspace { name: "u".into() })
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
            label: Some("u".into()),
            capabilities: vec!["workspace:read".into(), "thread:transition".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());

    // Park t1 with a reason.
    let put = client
        .put(format!("{base}/threads/{}/unclaimable", t1.id.0))
        .header("Authorization", &auth)
        .json(&serde_json::json!({ "reason": "needs triage" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    assert_eq!(
        put.json::<serde_json::Value>().await.unwrap()["reason"],
        "needs triage"
    );

    // An empty reason is a 400.
    let bad = client
        .put(format!("{base}/threads/{}/unclaimable", t2.id.0))
        .header("Authorization", &auth)
        .json(&serde_json::json!({ "reason": "   " }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    // The channel list shows the one parked thread.
    let listed: serde_json::Value = client
        .get(format!("{base}/channels/{}/unclaimable", ch.id.0))
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

    let claim = |thread: uuid::Uuid| {
        let (client, base, auth) = (client.clone(), base.clone(), auth.clone());
        async move {
            client
                .post(format!("{base}/threads/{thread}/assignee/claim"))
                .header("Authorization", &auth)
                .json(&serde_json::json!({ "member_id": agent.id.0 }))
                .send()
                .await
                .unwrap()
        }
    };

    // An explicit claim of the parked thread is refused (409).
    assert_eq!(claim(t1.id.0).await.status(), StatusCode::CONFLICT);

    // claim_next skips the older parked t1 and claims t2.
    let next: serde_json::Value = client
        .post(format!("{base}/channels/{}/threads/claim-next", ch.id.0))
        .header("Authorization", &auth)
        .json(&serde_json::json!({ "member_id": agent.id.0 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(next["id"], t2.id.0.to_string(), "the parked t1 is skipped");

    // Un-parking: 204 the first time, 404 the second (idempotent).
    assert_eq!(
        client
            .delete(format!("{base}/threads/{}/unclaimable", t1.id.0))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client
            .delete(format!("{base}/threads/{}/unclaimable", t1.id.0))
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
