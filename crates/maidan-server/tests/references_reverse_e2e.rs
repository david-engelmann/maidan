//! Cluster 320: reverse-edge + by-type reference queries. `GET /references` now
//! lists FROM a source or TO a target, optionally filtered by relation.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, reqwest::Client) {
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
    std::mem::forget(dir);
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    (addr, client)
}

#[tokio::test]
async fn references_query_by_source_target_and_relation() {
    let (addr, client) = spawn().await;
    let base = format!("http://{addr}");

    let post = |path: String, body: Value| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .post(format!("{base}{path}"))
                .json(&body)
                .send()
                .await
                .unwrap()
        }
    };

    let ws: Value = post("/workspaces".into(), json!({"name": "refs"}))
        .await
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();
    let ch: Value = post(
        format!("/workspaces/{wid}/channels"),
        json!({"name": "c", "private": false}),
    )
    .await
    .json()
    .await
    .unwrap();
    let cid = ch["id"].as_str().unwrap();
    let mk_thread = |title: &str| {
        let p = post(format!("/channels/{cid}/threads"), json!({"title": title}));
        async move {
            p.await.json::<Value>().await.unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let src_a = mk_thread("A").await;
    let src_b = mk_thread("B").await;
    let target = mk_thread("target").await;

    // A --supports--> target, B --refutes--> target.
    for (src, rel) in [(&src_a, "supports"), (&src_b, "refutes")] {
        let r = post(
            "/references".into(),
            json!({"src_kind":"thread","src_id":src,"dst_kind":"thread","dst_id":target,"relation":rel}),
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);
    }

    let list = |query: String| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .get(format!("{base}/references?{query}"))
                .send()
                .await
                .unwrap()
        }
    };

    // Reverse edge: what references `target`? Both A and B.
    let to: Value = list(format!("dst_kind=thread&dst_id={target}"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        to.as_array().unwrap().len(),
        2,
        "reverse query returns both edges"
    );

    // By-type: only the refutes edge.
    let refutes: Value = list(format!("dst_kind=thread&dst_id={target}&relation=refutes"))
        .await
        .json()
        .await
        .unwrap();
    let refutes = refutes.as_array().unwrap();
    assert_eq!(refutes.len(), 1);
    assert_eq!(refutes[0]["relation"], "refutes");
    assert_eq!(refutes[0]["src_id"], src_b.as_str());

    // Forward edge still works: references FROM A.
    let from: Value = list(format!("src_kind=thread&src_id={src_a}"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(from.as_array().unwrap().len(), 1);
    assert_eq!(from.as_array().unwrap()[0]["relation"], "supports");

    // Neither pair → 400.
    let bad = list("relation=supports".into()).await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}
