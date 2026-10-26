//! A thread's digest over REST and MCP, with auth on and two workspaces. One
//! agent uploads a summary as a `digest` artifact and links it; a reader gets
//! the pointer, then the bytes by sha256. Another workspace reaches neither.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId,
    WorkspaceId,
};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
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
    (addr, reqwest::Client::new(), store)
}

/// A workspace with one member, its token, and one thread.
async fn tenant(store: &dyn Store, name: &str) -> (WorkspaceId, MemberId, String, ThreadId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "worker".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id;
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
                capability::ARTIFACT_UPLOAD.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    (ws, member, format!("Bearer {}", secret.as_str()), thread.id)
}

async fn call(
    client: &reqwest::Client,
    method: Method,
    url: String,
    token: &str,
) -> (StatusCode, Value) {
    let resp = client
        .request(method, url)
        .header("Authorization", token)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

async fn upload(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    kind: &str,
    body: &str,
) -> String {
    let resp: Value = client
        .post(format!("{base}/artifacts?kind={kind}"))
        .header("Authorization", token)
        .body(body.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    resp["sha256"]
        .as_str()
        .unwrap_or_else(|| panic!("upload {kind}: {resp}"))
        .to_string()
}

async fn mcp_read(client: &reqwest::Client, base: &str, token: &str, uri: &str) -> Value {
    client
        .post(format!("{base}/mcp"))
        .header("Authorization", token)
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "resources/read",
            "params": {"uri": uri}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_linked_digest_is_read_once_by_hash_and_stays_in_its_workspace() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let (_, _, token, thread) = tenant(store.as_ref(), "a").await;
    let (_, _, other_token, _) = tenant(store.as_ref(), "b").await;
    let t = thread.0;

    let (status, _) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{t}/digest"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no digest yet");

    let notes = upload(&client, &base, &token, "transcript", "raw notes").await;
    let summary = "# Summary\n\nThe fix is in; review the migration.";
    let sha = upload(
        &client,
        &base,
        &token,
        "digest&mime_type=text/markdown",
        summary,
    )
    .await;
    for linked in [&notes, &sha] {
        let (status, body) = call(
            &client,
            Method::PUT,
            format!("{base}/threads/{t}/artifacts/{linked}"),
            &token,
        )
        .await;
        assert!(status.is_success(), "link {linked}: {status} {body}");
    }

    let (status, digest) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{t}/digest"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{digest}");
    assert_eq!(digest["sha256"], sha.as_str());
    assert_eq!(digest["uri"], format!("maidan://artifacts/{sha}"));
    assert_eq!(digest["mime_type"], "text/markdown");
    assert_eq!(digest["size_bytes"].as_i64(), Some(summary.len() as i64));

    let bytes = client
        .get(format!("{base}/artifacts/{sha}"))
        .header("Authorization", &token)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(bytes, summary, "the pointer names the bytes");

    let read = mcp_read(&client, &base, &token, &format!("maidan://digests/{t}")).await;
    let text = read["result"]["contents"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{read}"));
    let via_mcp: Value = serde_json::from_str(text).unwrap();
    assert_eq!(via_mcp, digest, "MCP and REST agree");
    assert_eq!(read["result"]["ttlMs"].as_u64(), Some(0));
    assert_eq!(read["result"]["cacheScope"], "private");

    let (status, _) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{t}/digest"),
        &other_token,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "another workspace cannot reach the thread"
    );
    let read = mcp_read(
        &client,
        &base,
        &other_token,
        &format!("maidan://digests/{t}"),
    )
    .await;
    assert!(
        read["error"].is_object(),
        "another workspace reads no digest: {read}"
    );
}
