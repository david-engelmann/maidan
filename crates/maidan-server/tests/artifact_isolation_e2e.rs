//! Cluster 204: artifacts are content-addressed + deduped across workspaces, so
//! this proves a caller in workspace B cannot fetch a blob workspace A uploaded
//! just by knowing its SHA — the `maidan_artifact_refs` per-tenant access gate.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

struct Ctx {
    addr: SocketAddr,
    _server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}
impl Ctx {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }
}

async fn spawn() -> Ctx {
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
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
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ctx {
        addr,
        _server: server,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        store,
        _dir: dir,
    }
}

/// A workspace + a token that can upload + read artifacts.
async fn workspace_with_token(ctx: &Ctx, name: &str) -> (WorkspaceId, String) {
    let ws = ctx
        .store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = ctx
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("m-{name}"),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    ctx.store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::ARTIFACT_UPLOAD.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws.id, secret.as_str().to_string())
}

fn bearer(t: &str) -> String {
    format!("Bearer {t}")
}

#[tokio::test]
async fn a_workspace_cannot_fetch_another_tenants_artifact_by_sha() {
    let ctx = spawn().await;
    let base = ctx.base();
    let (_wa, tok_a) = workspace_with_token(&ctx, "acme").await;
    let (_wb, tok_b) = workspace_with_token(&ctx, "other").await;

    let payload = b"secret tenant-a bytes".to_vec();

    // A uploads.
    let created_resp = ctx
        .client
        .post(format!("{base}/artifacts?kind=attachment"))
        .header("Authorization", bearer(&tok_a))
        .body(payload.clone())
        .send()
        .await
        .unwrap();
    let status = created_resp.status();
    let created: Value = created_resp.json().await.unwrap();
    assert_eq!(status, StatusCode::CREATED, "upload failed: {created}");
    let sha = created["sha256"].as_str().unwrap().to_string();

    // A can fetch its own artifact (blob + metadata).
    for suffix in ["", "/meta"] {
        let resp = ctx
            .client
            .get(format!("{base}/artifacts/{sha}{suffix}"))
            .header("Authorization", bearer(&tok_a))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "owner reads {suffix}");
    }

    // B knows the SHA but has no access link → 404 (indistinguishable from absent).
    for suffix in ["", "/meta"] {
        let resp = ctx
            .client
            .get(format!("{base}/artifacts/{sha}{suffix}"))
            .header("Authorization", bearer(&tok_b))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "cross-tenant fetch of {suffix} is denied"
        );
    }

    // Once B uploads the SAME bytes, it gets its own ref and may fetch the
    // (deduped) blob — legitimately.
    let reupload = ctx
        .client
        .post(format!("{base}/artifacts?kind=attachment"))
        .header("Authorization", bearer(&tok_b))
        .body(payload)
        .send()
        .await
        .unwrap();
    assert_eq!(reupload.status(), StatusCode::CREATED);
    let b_reads = ctx
        .client
        .get(format!("{base}/artifacts/{sha}"))
        .header("Authorization", bearer(&tok_b))
        .send()
        .await
        .unwrap();
    assert_eq!(
        b_reads.status(),
        StatusCode::OK,
        "B may read the blob it also uploaded"
    );
}
