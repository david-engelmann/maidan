//! WIP limit (Cluster 362, G11): the admin/visibility API plus enforcement on the
//! explicit claim (409) and `claim_next` (silent null).

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn wip_limit_blocks_claim_and_claim_next_over_the_cap() {
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
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "wip".into() })
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
            label: Some("wip".into()),
            capabilities: vec![
                "workspace:read".into(),
                "workspace:write".into(),
                "thread:transition".into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());

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

    // Cap the workspace at one concurrent live claim per member.
    let put = client
        .put(format!("{base}/workspaces/{}/wip-limit", ws.id.0))
        .header("Authorization", &auth)
        .json(&serde_json::json!({ "limit": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    assert_eq!(put.json::<serde_json::Value>().await.unwrap()["limit"], 1);

    let wip = |member: uuid::Uuid| {
        let (client, base, auth) = (client.clone(), base.clone(), auth.clone());
        async move {
            client
                .get(format!("{base}/members/{member}/wip"))
                .header("Authorization", &auth)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        }
    };
    let before = wip(agent.id.0).await;
    assert_eq!(before["live_claims"], 0);
    assert_eq!(before["limit"], 1);

    // First claim succeeds; the member is now at the cap.
    assert_eq!(claim(t1.id.0).await.status(), StatusCode::OK);
    assert_eq!(wip(agent.id.0).await["live_claims"], 1);

    // A second explicit claim is refused with 409 (agent busy).
    assert_eq!(claim(t2.id.0).await.status(), StatusCode::CONFLICT);

    // claim_next hands out nothing (silent null) while at the cap.
    let next = client
        .post(format!("{base}/channels/{}/threads/claim-next", ch.id.0))
        .header("Authorization", &auth)
        .json(&serde_json::json!({ "member_id": agent.id.0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(next.status(), StatusCode::OK);
    assert!(next.json::<serde_json::Value>().await.unwrap().is_null());

    // Lifting the cap lets the second claim through.
    client
        .put(format!("{base}/workspaces/{}/wip-limit", ws.id.0))
        .header("Authorization", &auth)
        .json(&serde_json::json!({ "limit": serde_json::Value::Null }))
        .send()
        .await
        .unwrap();
    assert_eq!(claim(t2.id.0).await.status(), StatusCode::OK);
    assert_eq!(wip(agent.id.0).await["live_claims"], 2);
    assert!(wip(agent.id.0).await["limit"].is_null());

    server.abort();
}
