//! Workspace purge deletes artifact blobs from LocalFs.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, LocalFsStore};
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
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
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::TOKEN_ADMIN.into()],
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

/// Artifacts are content-addressed and shared across workspaces: one row and
/// one blob per sha, whoever uploaded it first. Purging a workspace used to
/// delete every artifact its members had uploaded — including bytes another
/// workspace had uploaded too — and then the blob, so the other tenant kept an
/// access link to content that no longer existed. A single-tenant test could
/// not see it.
#[tokio::test]
async fn purging_one_workspace_never_destroys_another_workspaces_artifact() {
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
    let state = AppState::new(
        store.clone(),
        artifacts.clone(),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    let client = reqwest::Client::new();

    // Two tenants, each with an admin token.
    let mut tenants = Vec::new();
    for name in ["first", "second"] {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "admin".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let secret = TokenSecret::generate();
        store
            .create_api_token(NewApiToken {
                workspace_id: ws.id,
                member_id: member.id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: vec![capability::TOKEN_ADMIN.into()],
                expires_at: None,
            })
            .await
            .unwrap();
        tenants.push((ws, member, secret.as_str().to_string()));
    }
    let (first, first_member, first_token) = &tenants[0];
    let (second, _, second_token) = &tenants[1];

    // Both upload the same bytes. The row keeps the first uploader.
    let sha = artifacts
        .put(Bytes::from_static(b"shared bytes"))
        .await
        .unwrap();
    store
        .upsert_artifact(NewArtifact {
            sha256: sha.to_hex(),
            size_bytes: 12,
            mime_type: Some("text/plain".into()),
            kind: ArtifactKind::Attachment,
            uploaded_by: Some(first_member.id),
        })
        .await
        .unwrap();
    for ws in [first, second] {
        store
            .record_artifact_ref(ws.id, &sha.to_hex())
            .await
            .unwrap();
    }

    let purge = |ws: maidan_types::WorkspaceId, token: String| {
        let client = client.clone();
        async move {
            let resp = client
                .post(format!("http://{addr}/workspaces/{}/purge", ws.0))
                .bearer_auth(token)
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            resp.json::<serde_json::Value>().await.unwrap()
        }
    };

    // The first uploader purges: its access goes, the second tenant's stays.
    let body = purge(first.id, first_token.clone()).await;
    assert_eq!(body["artifacts_removed"], 0, "{body}");
    assert!(
        artifacts.exists(&sha).await.unwrap(),
        "the blob must survive"
    );
    assert!(store.get_artifact_by_sha(&sha.to_hex()).await.is_ok());
    assert!(!store
        .artifact_ref_exists(first.id, &sha.to_hex())
        .await
        .unwrap());
    assert!(store
        .artifact_ref_exists(second.id, &sha.to_hex())
        .await
        .unwrap());

    // The last tenant referencing it purges: now it is truly gone.
    let body = purge(second.id, second_token.clone()).await;
    assert_eq!(body["artifacts_removed"], 1, "{body}");
    assert!(!artifacts.exists(&sha).await.unwrap());
    assert!(store.get_artifact_by_sha(&sha.to_hex()).await.is_err());

    server.abort();
}
