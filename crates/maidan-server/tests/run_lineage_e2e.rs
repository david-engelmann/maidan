//! REST run lineage. A PUT result carrying the pi fixture `run_id` homes
//! `parent_run_id` on the thread (no minted id). Nested occupancy attributes a
//! child that shares the value. F7 mute is orthogonal. Auth ENABLED (real
//! token).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const PI_RUN_ID: &str = "aa4dc966-0e09-44c3-b7a5-2d048b48b301";
const FIXTURE: &str = include_str!("../../maidan-types/tests/fixtures/waiter_result_v1.json");

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
async fn result_put_homes_fixture_run_id_and_nested_occupancy_is_attributed() {
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

    let ws = store
        .create_workspace(NewWorkspace {
            name: "lineage".into(),
        })
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
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let parent = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parent".into()),
        })
        .await
        .unwrap();
    let child = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: Some(parent.id),
            title: Some("nested".into()),
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;

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
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let bearer = format!("Bearer {tok}");

    let miss = client
        .get(format!("{base}/threads/{}/lineage", parent.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(miss.status(), StatusCode::NOT_FOUND);

    let fixture: Value = serde_json::from_str(FIXTURE).expect("fixture");
    assert_eq!(fixture["run_id"], json!(PI_RUN_ID));

    let set = client
        .put(format!("{base}/threads/{}/result", parent.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "result": fixture }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);

    let lined: Value = client
        .get(format!("{base}/threads/{}/lineage", parent.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(lined["parent_run_id"], json!(PI_RUN_ID));
    assert_eq!(lined["thread_id"], json!(parent.id.0));

    let nest = client
        .put(format!("{base}/threads/{}/lineage", child.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "parent_run_id": PI_RUN_ID }))
        .send()
        .await
        .unwrap();
    assert_eq!(nest.status(), StatusCode::OK);

    let listed: Value = client
        .get(format!(
            "{base}/workspaces/{}/run-threads?parent_run_id={PI_RUN_ID}",
            ws.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<String> = listed
        .as_array()
        .expect("array")
        .iter()
        .map(|t| t["id"].as_str().expect("id").to_string())
        .collect();
    assert_eq!(
        ids,
        vec![parent.id.0.to_string(), child.id.0.to_string()],
        "nested child is attributed to the producer run"
    );

    let occ: Value = client
        .get(format!(
            "{base}/workspaces/{}/run-occupancy?parent_run_id={PI_RUN_ID}",
            ws.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(occ["parent_run_id"], json!(PI_RUN_ID));
    assert_eq!(occ["open"], json!(2));
    assert_eq!(occ["queued"], json!(2));

    store.mute_thread(member.id, child.id).await.unwrap();
    let occ_muted: Value = client
        .get(format!(
            "{base}/workspaces/{}/run-occupancy?parent_run_id={PI_RUN_ID}",
            ws.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        occ_muted["open"],
        json!(2),
        "F7 mute is orthogonal — muted nested work still counts"
    );

    let blank = client
        .put(format!("{base}/threads/{}/lineage", child.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "parent_run_id": "   " }))
        .send()
        .await
        .unwrap();
    assert_eq!(blank.status(), StatusCode::BAD_REQUEST);

    let cleared = client
        .delete(format!("{base}/threads/{}/lineage", child.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(cleared.status(), StatusCode::NO_CONTENT);
    let gone = client
        .get(format!("{base}/threads/{}/lineage", child.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}
