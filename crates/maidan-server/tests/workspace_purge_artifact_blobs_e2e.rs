//! Workspace purge deletes artifact blobs from LocalFs (Cluster 31).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, LocalFsStore};
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{ArtifactKind, MemberKind, NewApiToken, NewArtifact, NewMember, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn purge_workspace_deletes_uploaded_artifact_blob() {
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
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts.clone(),
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "artifact-purge".into(),
        })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let payload = Bytes::from_static(b"purge-me");
    let sha = artifacts.put(payload).await.unwrap();
    store
        .upsert_artifact(NewArtifact {
            sha256: sha.to_hex(),
            size_bytes: 8,
            mime_type: Some("text/plain".into()),
            kind: ArtifactKind::Attachment,
            uploaded_by: Some(alice.id),
        })
        .await
        .unwrap();
    assert!(artifacts.exists(&sha).await.unwrap());

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: alice.id,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_WRITE.into()],
            expires_at: None,
        })
        .await
        .unwrap();

    let resp = client
        .post(format!("{base}/workspaces/{}/purge", ws.id.0))
        .header("authorization", format!("Bearer {}", secret.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["artifacts_removed"], 1);
    assert!(!artifacts.exists(&sha).await.unwrap());

    server.abort();
}
