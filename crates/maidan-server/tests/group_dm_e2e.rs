//! Cluster 97: multi-member group DM threads.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    Arc<dyn Store>,
) {
    let pool = SqlitePoolOptions::new()
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
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
    let bus = Arc::new(InMemoryBus::new());
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
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), server, store)
}

async fn mint_token(store: &dyn Store, workspace_id: WorkspaceId) -> String {
    let member = store
        .create_member(NewMember {
            workspace_id,
            handle: format!("bot-{}", uuid::Uuid::new_v4()),
            display_name: None,
            kind: maidan_types::MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn group_dm_open_post_and_list() {
    let (addr, client, server, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "group-dm".into(),
        })
        .await
        .unwrap();
    let a = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();
    let b = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "b".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();
    let c = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "c".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();
    let token = mint_token(store.as_ref(), ws.id).await;

    let open = client
        .post(format!("{base}/workspaces/{}/group-dms", ws.id))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "member_ids": [a.id.0, b.id.0, c.id.0],
            "title": "trio"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(open.status(), StatusCode::CREATED);
    let group: serde_json::Value = open.json().await.unwrap();
    assert_eq!(group["member_ids"].as_array().unwrap().len(), 3);
    let gid = group["id"].as_str().unwrap();

    let msg = client
        .post(format!("{base}/group-dms/{gid}/messages"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "author_id": a.id.0, "body": "hello group" }))
        .send()
        .await
        .unwrap();
    assert_eq!(msg.status(), StatusCode::CREATED);

    let list = client
        .get(format!(
            "{base}/workspaces/{}/group-dms?member_id={}",
            ws.id, a.id.0
        ))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let rows: Vec<serde_json::Value> = list.json().await.unwrap();
    assert_eq!(rows.len(), 1);

    server.abort();
}
