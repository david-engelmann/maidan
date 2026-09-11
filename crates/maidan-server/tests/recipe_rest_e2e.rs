//! Recipe management over HTTP (Cluster 370.3, Wave 2 #18): create / list / get /
//! delete + instantiate. Runs with auth ENABLED so `created_by` is a real member
//! and channel access is exercised (the created_by FK gotcha).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace, WorkspaceId,
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
async fn recipe_crud_and_instantiate_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
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
    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let bearer = format!("Bearer {tok}");

    let spec = json!({
        "params": [{ "name": "repo", "required": true }],
        "definition_of_done": "PR merged",
        "children": [
            { "key": "build", "title": "build it", "required_skills": ["rust"] },
            { "key": "review", "title": "review it", "depends_on": ["build"] }
        ]
    });

    // Create.
    let resp = client
        .post(format!("{base}/workspaces/{}/recipes", ws.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "channel_id": channel.id.0, "name": "ship", "spec": spec }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: Value = resp.json().await.unwrap();
    assert_eq!(created["name"], json!("ship"));
    let recipe_id = created["id"].as_str().unwrap().to_string();

    // List + get.
    let list: Value = client
        .get(format!("{base}/workspaces/{}/recipes", ws.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    let got: Value = client
        .get(format!("{base}/workspaces/{}/recipes/{recipe_id}", ws.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["spec"]["children"].as_array().unwrap().len(), 2);

    // Instantiate with the required param → a run whose root is a real thread.
    let run: Value = client
        .post(format!(
            "{base}/workspaces/{}/recipes/{recipe_id}/instantiate",
            ws.id.0
        ))
        .header("Authorization", &bearer)
        .json(&json!({ "params": { "repo": "x/y" } }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(run["recipe_id"], json!(recipe_id));
    let root_id = run["root_thread_id"].as_str().unwrap();
    // The root thread exists and carries two children.
    let children: Value = client
        .get(format!("{base}/threads/{root_id}/children"))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(children.as_array().unwrap().len(), 2);

    // Instantiating without the required param is a 400.
    let bad = client
        .post(format!(
            "{base}/workspaces/{}/recipes/{recipe_id}/instantiate",
            ws.id.0
        ))
        .header("Authorization", &bearer)
        .json(&json!({ "params": {} }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    // A cyclic/invalid spec is rejected at create (400).
    let cyclic = client
        .post(format!("{base}/workspaces/{}/recipes", ws.id.0))
        .header("Authorization", &bearer)
        .json(&json!({
            "channel_id": channel.id.0,
            "name": "bad",
            "spec": { "children": [
                { "key": "a", "title": "a", "depends_on": ["b"] },
                { "key": "b", "title": "b", "depends_on": ["a"] }
            ] }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cyclic.status(), StatusCode::BAD_REQUEST);

    // Delete → gone (404 on repeat get).
    let del = client
        .delete(format!("{base}/workspaces/{}/recipes/{recipe_id}", ws.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let gone = client
        .get(format!("{base}/workspaces/{}/recipes/{recipe_id}", ws.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}
