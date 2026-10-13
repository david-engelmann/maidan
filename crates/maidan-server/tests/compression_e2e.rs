//! Response compression over HTTP: a client that offers gzip gets the
//! big JSON documents encoded, byte-for-byte the identity body once
//! decoded; a client that does not ask is answered unencoded.
//!
//! The requests are hand-written over a raw socket: reqwest would decode
//! transparently and hide exactly what this test exists to see.

use std::io::Read as _;
use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use sqlx::sqlite::SqlitePoolOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn spawn() -> String {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
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
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
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
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("{addr}")
}

/// One raw HTTP/1.1 GET. Returns the status line, the headers lowercased
/// into (name, value) pairs, and the body with chunked framing removed.
async fn get(
    addr: &str,
    path: &str,
    accept_encoding: Option<&str>,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut request = format!("GET {path} HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n");
    if let Some(encoding) = accept_encoding {
        request.push_str(&format!("accept-encoding: {encoding}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response has a head");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut body = raw[split + 4..].to_vec();
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("status line parses");
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    if header(&headers, "transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        body = dechunk(&body);
    }
    (status, headers, body)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let line_end = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .expect("chunk size line");
        let size = usize::from_str_radix(
            std::str::from_utf8(&rest[..line_end]).expect("chunk size is ascii"),
            16,
        )
        .expect("chunk size is hex");
        rest = &rest[line_end + 2..];
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

fn gunzip(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(body)
        .read_to_end(&mut out)
        .expect("body is one gzip stream");
    out
}

async fn assert_gzipped_round_trip(addr: &str, path: &str, ratio: usize) {
    let (status, headers, identity) = get(addr, path, None).await;
    assert_eq!(status, 200, "{path} answers");
    assert!(
        header(&headers, "content-encoding").is_none(),
        "a client that does not ask gets identity: {headers:?}"
    );

    let (status, headers, encoded) = get(addr, path, Some("gzip")).await;
    assert_eq!(status, 200, "{path} answers a gzip client");
    assert_eq!(
        header(&headers, "content-encoding"),
        Some("gzip"),
        "the answer is encoded: {headers:?}"
    );
    assert!(
        header(&headers, "vary")
            .is_some_and(|v| v.to_ascii_lowercase().contains("accept-encoding")),
        "caches are told the answer varies by encoding: {headers:?}"
    );
    assert_eq!(
        gunzip(&encoded),
        identity,
        "the encoded body decodes to the very bytes the identity answer carries"
    );
    assert!(
        encoded.len() * ratio < identity.len(),
        "the encoding earns its keep on {path}: {} bytes in, {} out",
        identity.len(),
        encoded.len()
    );
}

#[tokio::test]
async fn the_openapi_document_is_gzipped_when_the_client_asks() {
    let addr = spawn().await;
    assert_gzipped_round_trip(&addr, "/openapi.json", 2).await;
}

#[tokio::test]
async fn the_agent_card_is_gzipped_when_the_client_asks() {
    let addr = spawn().await;
    assert_gzipped_round_trip(&addr, "/.well-known/agent-card.json", 1).await;
}

/// A response that carries a credential is never compressed: connecting an
/// agent answers with its token beside the handle the caller chose, so a
/// compressed body's length would be an oracle on the secret.
#[tokio::test]
async fn a_credential_response_is_no_store_and_never_compressed() {
    use maidan_auth::{capability, hash_secret, TokenSecret};
    use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};

    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let ws = store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let admin = store
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
            member_id: admin.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::TOKEN_ADMIN.into(),
                capability::WORKSPACE_READ.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
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
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let body = format!(
        "{{\"handle\":\"builder-{}\",\"display_name\":\"a display name long enough to compress well\"}}",
        "x".repeat(40)
    );
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "POST /workspaces/{}/agents HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n\
         authorization: Bearer {}\r\ncontent-type: application/json\r\n\
         accept-encoding: gzip\r\ncontent-length: {}\r\n\r\n{body}",
        ws.id.0,
        secret.as_str(),
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a header block");
    let head = String::from_utf8_lossy(&raw[..head_end]).to_lowercase();
    assert!(head.starts_with("http/1.1 201"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
    assert!(
        !head.contains("content-encoding"),
        "a credential response is never compressed: {head}"
    );
}
