//! Artifact bytes are served as the server decided, not as the bytes or the
//! request suggest: the type stored at upload (or `application/octet-stream`),
//! `nosniff` always, a sandboxing CSP, and `inline` only for raster images.
//! And an upload's filename is its workspace's: a tenant holding the same
//! deduplicated bytes never sees another tenant's name for them.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

const CSP: &str = "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; sandbox";

struct Ctx {
    base: String,
    _server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ctx {
        base: format!("http://{addr}"),
        _server: server,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        store,
        _dir: dir,
    }
}

async fn tenant_token(ctx: &Ctx, name: &str) -> String {
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
    secret.as_str().to_string()
}

async fn upload(ctx: &Ctx, token: &str, query: &[(&str, &str)], body: &[u8]) -> Value {
    let mut params = vec![("kind", "attachment")];
    params.extend_from_slice(query);
    let resp = ctx
        .client
        .post(format!("{}/artifacts", ctx.base))
        .query(&params)
        .bearer_auth(token)
        .body(body.to_vec())
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(status, StatusCode::CREATED, "upload {query:?}: {body}");
    body
}

struct Served {
    content_type: Option<String>,
    disposition: Option<String>,
    nosniff: Option<String>,
    csp: Option<String>,
    cache: Option<String>,
}

async fn fetch(ctx: &Ctx, token: &str, path: &str) -> Served {
    let resp = ctx
        .client
        .get(format!("{}{path}", ctx.base))
        .bearer_auth(token)
        // A request that asks for HTML must not get it.
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{path}");
    let header = |name: &str| {
        resp.headers()
            .get(name)
            .map(|v| v.to_str().unwrap().to_owned())
    };
    Served {
        content_type: header("content-type"),
        disposition: header("content-disposition"),
        nosniff: header("x-content-type-options"),
        csp: header("content-security-policy"),
        cache: header("cache-control"),
    }
}

#[tokio::test]
async fn each_type_is_served_with_the_headers_the_server_decided() {
    let ctx = spawn().await;
    let token = tenant_token(&ctx, "acme").await;
    // (stored type, served type, inline). An SVG is a document that can carry
    // script, so it downloads like HTML does.
    let cases: [(Option<&str>, &str, bool); 10] = [
        (Some("image/png"), "image/png", true),
        (Some("image/jpeg"), "image/jpeg", true),
        (Some("image/gif"), "image/gif", true),
        (Some("IMAGE/WEBP; q=1"), "image/webp", true),
        (Some("image/svg+xml"), "image/svg+xml", false),
        (Some("text/html"), "text/html", false),
        (Some("application/pdf"), "application/pdf", false),
        (
            Some("text/html, image/png"),
            "application/octet-stream",
            false,
        ),
        (
            Some("definitely not a type"),
            "application/octet-stream",
            false,
        ),
        (None, "application/octet-stream", false),
    ];
    for (i, (stored, served, inline)) in cases.into_iter().enumerate() {
        let body = format!("<svg onload=alert({i})><script>alert({i})</script></svg>");
        let mut query = vec![("filename", "f.bin")];
        if let Some(stored) = stored {
            query.push(("mime_type", stored));
        }
        let sha = upload(&ctx, &token, &query, body.as_bytes()).await["sha256"]
            .as_str()
            .unwrap()
            .to_string();
        // The bearer route and the console's session route answer alike.
        for path in [
            format!("/artifacts/{sha}"),
            format!("/ui/api/artifacts/{sha}"),
        ] {
            let got = fetch(&ctx, &token, &path).await;
            assert_eq!(
                got.content_type.as_deref(),
                Some(served),
                "{stored:?} {path}"
            );
            assert_eq!(got.nosniff.as_deref(), Some("nosniff"), "{stored:?}");
            assert_eq!(got.csp.as_deref(), Some(CSP), "{stored:?}");
            assert_eq!(got.cache.as_deref(), Some("private"), "{stored:?}");
            let disposition = if inline { "inline" } else { "attachment" };
            assert_eq!(
                got.disposition.as_deref(),
                Some(format!("{disposition}; filename=\"f.bin\"; filename*=UTF-8''f.bin").as_str()),
                "{stored:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_tenant_sees_its_own_filename_for_shared_bytes_never_anothers() {
    let ctx = spawn().await;
    let tok_a = tenant_token(&ctx, "acme").await;
    let tok_b = tenant_token(&ctx, "bravo").await;
    let tok_c = tenant_token(&ctx, "charlie").await;
    let bytes = b"the same bytes in every tenant";

    let a = upload(
        &ctx,
        &tok_a,
        &[
            ("filename", "acme-acquisition-plan.png"),
            ("mime_type", "image/png"),
        ],
        bytes,
    )
    .await;
    let sha = a["sha256"].as_str().unwrap().to_string();
    assert_eq!(a["filename"], "acme-acquisition-plan.png");
    let b = upload(
        &ctx,
        &tok_b,
        &[("filename", "notes.txt"), ("mime_type", "text/plain")],
        bytes,
    )
    .await;
    assert_eq!(b["sha256"], sha.as_str(), "deduplicated");
    assert_eq!(b["filename"], "notes.txt");
    let c = upload(&ctx, &tok_c, &[], bytes).await;
    assert!(c.get("filename").is_none(), "C named nothing: {c}");

    for (token, name, disposition) in [
        (&tok_a, Some("acme-acquisition-plan.png"), "inline"),
        (&tok_b, Some("notes.txt"), "attachment"),
        (&tok_c, None, "attachment"),
    ] {
        let meta: Value = ctx
            .client
            .get(format!("{}/artifacts/{sha}/meta", ctx.base))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(meta.get("filename").and_then(Value::as_str), name, "{meta}");
        let got = fetch(&ctx, token, &format!("/artifacts/{sha}")).await;
        let disposition_header = got.disposition.unwrap();
        assert!(
            disposition_header.starts_with(disposition),
            "{disposition_header}"
        );
        if name != Some("acme-acquisition-plan.png") {
            assert!(
                !disposition_header.contains("acme"),
                "another tenant's filename leaked: {disposition_header}"
            );
        }
    }
}

#[tokio::test]
async fn an_upload_filename_is_a_name_not_a_path() {
    let ctx = spawn().await;
    let token = tenant_token(&ctx, "acme").await;
    let a = upload(&ctx, &token, &[("filename", "../../etc/evil.png")], b"one").await;
    assert_eq!(a["filename"], "evil.png");
    let b = upload(&ctx, &token, &[("filename", "../")], b"two").await;
    assert!(b.get("filename").is_none(), "{b}");

    for hostile in ["a\r\nSet-Cookie: x=1.png", "photo\u{202E}gpj.exe"] {
        let resp = ctx
            .client
            .post(format!("{}/artifacts", ctx.base))
            .query(&[("kind", "attachment"), ("filename", hostile)])
            .bearer_auth(&token)
            .body("three")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{hostile:?}");
    }
}
