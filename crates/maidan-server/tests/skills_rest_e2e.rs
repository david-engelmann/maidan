//! Capability-registry REST (Cluster 232): declare/list/remove member skills and
//! thread required-skills. Auth ENABLED (real token) so RBAC is exercised.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace,
    WorkspaceId,
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
async fn skills_crud_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
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
            channel_id: ChannelId(channel.id.0),
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let bearer = format!("Bearer {tok}");

    // --- member skills ---
    let add = client
        .post(format!("{base}/members/{}/skills", member.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "skill": "rust" }))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::NO_CONTENT);

    let skills: Value = client
        .get(format!("{base}/members/{}/skills", member.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(skills.as_array().unwrap().len(), 1);
    assert_eq!(skills[0]["skill"], json!("rust"));

    let del = client
        .delete(format!("{base}/members/{}/skills/rust", member.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let del2 = client
        .delete(format!("{base}/members/{}/skills/rust", member.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del2.status(), StatusCode::NOT_FOUND);

    // Empty skill is rejected.
    let bad = client
        .post(format!("{base}/members/{}/skills", member.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "skill": "  " }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    // --- thread required skills ---
    let add = client
        .post(format!("{base}/threads/{}/required-skills", thread.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "skill": "code-review" }))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::NO_CONTENT);

    let reqs: Value = client
        .get(format!("{base}/threads/{}/required-skills", thread.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reqs.as_array().unwrap().len(), 1);
    assert_eq!(reqs[0]["skill"], json!("code-review"));

    let del = client
        .delete(format!(
            "{base}/threads/{}/required-skills/code-review",
            thread.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}
