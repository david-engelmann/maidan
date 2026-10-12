//! The media quota over REST and MCP, and media expiry through the retention
//! sweeper. A media upload (attachment, screenshot, recording) that would take
//! a workspace past its quota is a 409 `media-quota-exceeded` problem naming
//! the workspace's usage, its quota and the upload's size. Bytes the workspace
//! already holds cost nothing, an erase frees its space, evidence kinds are
//! not counted, and one workspace's media is not another's. The quota is set
//! per workspace through the retention route, since these tests share one
//! process and its environment.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine};
use maidan_artifacts::{ArtifactStore, LocalFsStore, Sha256};
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{retention, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Ctx {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    artifacts: Arc<dyn ArtifactStore>,
    pool: sqlx::SqlitePool,
    _dir: tempfile::TempDir,
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(LocalFsStore::new(dir.path()));
    let state = AppState::new(
        store.clone(),
        artifacts.clone(),
        Arc::new(maidan_bus::InMemoryBus::new()),
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
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ctx {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        artifacts,
        pool,
        _dir: dir,
    }
}

/// A workspace and a token that can upload, read, erase and set retention.
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
                capability::TOKEN_ADMIN.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws.id, secret.as_str().to_string())
}

async fn set_quota(ctx: &Ctx, ws: WorkspaceId, token: &str, bytes: i64) {
    let response = ctx
        .client
        .put(format!("{}/workspaces/{}/retention", ctx.base, ws.0))
        .bearer_auth(token)
        .json(&json!({ "media_quota_bytes": bytes }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let policy: Value = response.json().await.unwrap();
    assert_eq!(policy["effective"]["media_quota_bytes"], bytes);
}

async fn upload(ctx: &Ctx, token: &str, kind: &str, bytes: &[u8]) -> reqwest::Response {
    ctx.client
        .post(format!("{}/artifacts?kind={kind}", ctx.base))
        .bearer_auth(token)
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap()
}

async fn uploaded(ctx: &Ctx, token: &str, kind: &str, bytes: &[u8]) -> String {
    let response = upload(ctx, token, kind, bytes).await;
    assert_eq!(response.status(), StatusCode::CREATED, "{kind} upload");
    let artifact: Value = response.json().await.unwrap();
    artifact["sha256"].as_str().unwrap().to_string()
}

/// The refusal's three numbers, after checking it is the quota's problem.
async fn refused(response: reqwest::Response) -> (i64, i64, i64) {
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response.headers()["content-type"],
        "application/problem+json"
    );
    let problem: Value = response.json().await.unwrap();
    assert_eq!(
        problem["type"], "https://maidan.dev/problems/media-quota-exceeded",
        "{problem}"
    );
    assert_eq!(problem["status"], 409);
    (
        problem["used_bytes"].as_i64().unwrap(),
        problem["quota_bytes"].as_i64().unwrap(),
        problem["upload_bytes"].as_i64().unwrap(),
    )
}

#[tokio::test]
async fn a_media_upload_that_would_cross_the_quota_is_a_409_with_the_three_numbers() {
    let ctx = spawn().await;
    let (ws, token) = workspace_with_token(&ctx, "acme").await;
    set_quota(&ctx, ws, &token, 100).await;

    uploaded(&ctx, &token, "attachment", &[1u8; 60]).await;
    let response = upload(&ctx, &token, "screenshot", &[2u8; 60]).await;
    assert_eq!(refused(response).await, (60, 100, 60));
    assert!(
        !ctx.artifacts
            .exists(&Sha256::compute(&[2u8; 60]))
            .await
            .unwrap(),
        "a refused single-shot upload writes no bytes"
    );
    // Up to the quota exactly is allowed.
    uploaded(&ctx, &token, "recording", &[3u8; 40]).await;
}

#[tokio::test]
async fn re_uploading_bytes_the_workspace_already_holds_is_neither_refused_nor_double_counted() {
    let ctx = spawn().await;
    let (ws, token) = workspace_with_token(&ctx, "acme").await;
    set_quota(&ctx, ws, &token, 100).await;

    uploaded(&ctx, &token, "attachment", &[1u8; 60]).await;
    uploaded(&ctx, &token, "attachment", &[1u8; 60]).await;
    uploaded(&ctx, &token, "screenshot", &[1u8; 60]).await;
    let response = upload(&ctx, &token, "screenshot", &[2u8; 41]).await;
    assert_eq!(
        refused(response).await,
        (60, 100, 41),
        "the held bytes count once"
    );
    uploaded(&ctx, &token, "screenshot", &[2u8; 40]).await;
}

#[tokio::test]
async fn erasing_an_artifact_frees_its_space_for_the_next_upload() {
    let ctx = spawn().await;
    let (ws, token) = workspace_with_token(&ctx, "acme").await;
    set_quota(&ctx, ws, &token, 100).await;

    let sha = uploaded(&ctx, &token, "attachment", &[1u8; 60]).await;
    let response = upload(&ctx, &token, "attachment", &[2u8; 60]).await;
    refused(response).await;

    let erased = ctx
        .client
        .delete(format!("{}/artifacts/{sha}", ctx.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(erased.status(), StatusCode::OK);
    uploaded(&ctx, &token, "attachment", &[2u8; 60]).await;
}

#[tokio::test]
async fn a_non_media_kind_is_not_counted_against_the_quota() {
    let ctx = spawn().await;
    let (ws, token) = workspace_with_token(&ctx, "acme").await;
    set_quota(&ctx, ws, &token, 100).await;

    for kind in ["transcript", "code_dump", "context_snapshot"] {
        let mut bytes = kind.as_bytes().to_vec();
        bytes.resize(500, b'.');
        uploaded(&ctx, &token, kind, &bytes).await;
    }
    uploaded(&ctx, &token, "attachment", &[1u8; 100]).await;
    let response = upload(&ctx, &token, "attachment", &[2u8; 1]).await;
    assert_eq!(refused(response).await, (100, 100, 1));
}

#[tokio::test]
async fn another_workspaces_usage_does_not_count_against_this_one() {
    let ctx = spawn().await;
    let (wa, tok_a) = workspace_with_token(&ctx, "acme").await;
    let (wb, tok_b) = workspace_with_token(&ctx, "other").await;
    set_quota(&ctx, wa, &tok_a, 100).await;
    set_quota(&ctx, wb, &tok_b, 100).await;

    uploaded(&ctx, &tok_b, "recording", &[9u8; 90]).await;
    uploaded(&ctx, &tok_a, "recording", &[8u8; 100]).await;
    // B's bytes are new to A, so they count against A's quota, once A holds
    // them, and only then.
    let response = upload(&ctx, &tok_a, "recording", &[9u8; 90]).await;
    assert_eq!(refused(response).await, (100, 100, 90));
}

#[tokio::test]
async fn the_mcp_upload_tool_is_held_to_the_same_quota() {
    let ctx = spawn().await;
    let (ws, token) = workspace_with_token(&ctx, "acme").await;
    set_quota(&ctx, ws, &token, 100).await;
    uploaded(&ctx, &token, "attachment", &[1u8; 60]).await;

    let response: Value = ctx
        .client
        .post(format!("{}/mcp", ctx.base))
        .bearer_auth(&token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "upload_artifact",
                "arguments": {"kind": "screenshot", "content_base64": STANDARD.encode([2u8; 60])}
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let said = response.to_string();
    assert!(
        said.contains("media quota exceeded") && said.contains("60") && said.contains("100"),
        "{response}"
    );
    assert!(!ctx
        .artifacts
        .exists(&Sha256::compute(&[2u8; 60]))
        .await
        .unwrap());
}

#[tokio::test]
async fn the_retention_sweep_expires_unreferenced_media_and_reaps_its_bytes() {
    let ctx = spawn().await;
    let (wa, tok_a) = workspace_with_token(&ctx, "acme").await;
    let (wb, tok_b) = workspace_with_token(&ctx, "other").await;
    let only_a = uploaded(&ctx, &tok_a, "attachment", b"only in a").await;
    let shared = uploaded(&ctx, &tok_a, "screenshot", b"in both").await;
    uploaded(&ctx, &tok_b, "transcript", b"in both").await;
    let evidence = uploaded(&ctx, &tok_a, "transcript", b"evidence").await;
    sqlx::query(
        "UPDATE maidan_artifact_refs
         SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-10 days'),
             last_uploaded_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-10 days')",
    )
    .execute(&ctx.pool)
    .await
    .unwrap();

    let mut cfg = retention::config_from_env();
    cfg.media_days = Some(1);
    retention::sweep_once(&ctx.store, &ctx.artifacts, &cfg).await;

    let hex = |sha: &str| Sha256::from_hex(sha).unwrap();
    assert!(!ctx.store.artifact_ref_exists(wa, &only_a).await.unwrap());
    assert!(
        !ctx.artifacts.exists(&hex(&only_a)).await.unwrap(),
        "the last reference takes the bytes with it"
    );
    assert!(!ctx.store.artifact_ref_exists(wa, &shared).await.unwrap());
    assert!(
        ctx.store.artifact_ref_exists(wb, &shared).await.unwrap(),
        "B's ref is a transcript, which never expires"
    );
    assert!(
        ctx.artifacts.exists(&hex(&shared)).await.unwrap(),
        "B still holds the bytes"
    );
    assert!(ctx.store.artifact_ref_exists(wa, &evidence).await.unwrap());
    let audit = ctx.store.list_audit_for_workspace(wa, 50).await.unwrap();
    assert_eq!(
        audit
            .iter()
            .filter(|row| row.action == "artifact.expire" && row.actor_id.is_none())
            .count(),
        2
    );
}

#[tokio::test]
async fn a_workspace_may_not_set_more_media_than_the_instance_allows() {
    let ctx = spawn().await;
    let (ws, token) = workspace_with_token(&ctx, "acme").await;
    let instance: Value = ctx
        .client
        .get(format!("{}/workspaces/{}/retention", ctx.base, ws.0))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let days = instance["instance"]["media_days"].as_i64().unwrap();
    let quota = instance["instance"]["media_quota_bytes"].as_i64().unwrap();
    for (body, field) in [
        (json!({ "media_days": days + 1 }), "media_days"),
        (
            json!({ "media_quota_bytes": quota + 1 }),
            "media_quota_bytes",
        ),
        (json!({ "media_quota_bytes": 0 }), "media_quota_bytes"),
    ] {
        let response = ctx
            .client
            .put(format!("{}/workspaces/{}/retention", ctx.base, ws.0))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert!(response.text().await.unwrap().contains(field), "{body}");
    }
}
