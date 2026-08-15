//! Task-dependency DAG management over HTTP (Cluster 219): add/list dependencies +
//! dependents + remove, and the `ready` flag flipping as a dependency closes.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_fsm::ThreadAction;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        true,
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
async fn dependency_management_add_list_ready_dependents_remove() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "dag".into() })
        .await
        .unwrap();
    let actor = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
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
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
    };
    let task = store.create_thread(mk("task")).await.unwrap();
    let dep = store.create_thread(mk("dep")).await.unwrap();

    // Add the edge.
    let add = client
        .post(format!("{base}/threads/{}/dependencies", task.id.0))
        .json(&serde_json::json!({ "depends_on_thread_id": dep.id.0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::NO_CONTENT);

    // Self-dependency is a 400.
    let self_dep = client
        .post(format!("{base}/threads/{}/dependencies", task.id.0))
        .json(&serde_json::json!({ "depends_on_thread_id": task.id.0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(self_dep.status(), StatusCode::BAD_REQUEST);

    // List: one dependency, not ready (dep is Open).
    let list = client
        .get(format!("{base}/threads/{}/dependencies", task.id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let view: serde_json::Value = list.json().await.unwrap();
    assert_eq!(view["dependencies"].as_array().unwrap().len(), 1);
    assert_eq!(view["ready"], false);

    // Dependents of `dep` include `task`.
    let dependents = client
        .get(format!("{base}/threads/{}/dependents", dep.id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(dependents.status(), StatusCode::OK);
    let deps: Vec<serde_json::Value> = dependents.json().await.unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(
        deps[0]["thread_id"].as_str().unwrap(),
        task.id.0.to_string()
    );

    // Close `dep` -> task becomes ready.
    store
        .transition_thread(dep.id, actor.id, ThreadAction::StartReview)
        .await
        .unwrap();
    store
        .transition_thread(dep.id, actor.id, ThreadAction::Close)
        .await
        .unwrap();
    let list2: serde_json::Value = client
        .get(format!("{base}/threads/{}/dependencies", task.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list2["ready"], true);

    // Remove the edge; a second remove is 404.
    let rm = client
        .delete(format!(
            "{base}/threads/{}/dependencies/{}",
            task.id.0, dep.id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(rm.status(), StatusCode::NO_CONTENT);
    let rm2 = client
        .delete(format!(
            "{base}/threads/{}/dependencies/{}",
            task.id.0, dep.id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(rm2.status(), StatusCode::NOT_FOUND);
}
