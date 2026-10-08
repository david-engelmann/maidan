//! A thread's version and linked artifacts over REST and MCP, with auth on and
//! two workspaces. Linking moves the version; a hash the thread's workspace
//! does not hold is not found, even when another workspace holds it; another
//! workspace's caller reaches neither the version nor the links.

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

const HELD: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const ELSEWHERE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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

#[tokio::test]
async fn linking_evidence_moves_the_version_and_stays_in_its_workspace() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let (ws, worker, token, thread) = tenant(store.as_ref(), "a").await;
    let (other_ws, _, other_token, _) = tenant(store.as_ref(), "b").await;
    store
        .record_artifact_ref(ws, &HELD.to_ascii_lowercase())
        .await
        .unwrap();
    store
        .record_artifact_ref(other_ws, ELSEWHERE)
        .await
        .unwrap();
    let t = thread.0;

    let (s, v) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{t}/version"),
        &token,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!({"thread_id": t, "version": 0}));

    // A hash in any case links as its one lowercase form, and moves the version.
    let (s, link) = call(
        &client,
        Method::PUT,
        format!("{base}/threads/{t}/artifacts/{HELD}"),
        &token,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{link}");
    assert_eq!(link["sha256"], HELD.to_ascii_lowercase());
    assert_eq!(link["linked_by"], json!(worker.0));
    let (_, v) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{t}/version"),
        &token,
    )
    .await;
    assert_eq!(v["version"], 1);

    // Another workspace holds this hash; this one does not, so it is not found.
    let (s, _) = call(
        &client,
        Method::PUT,
        format!("{base}/threads/{t}/artifacts/{ELSEWHERE}"),
        &token,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(
        &client,
        Method::PUT,
        format!("{base}/threads/{t}/artifacts/not-a-hash"),
        &token,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Another workspace's caller reaches neither the version nor the links.
    for (method, path) in [
        (Method::GET, format!("/threads/{t}/version")),
        (Method::GET, format!("/threads/{t}/artifacts")),
        (Method::PUT, format!("/threads/{t}/artifacts/{ELSEWHERE}")),
        (Method::DELETE, format!("/threads/{t}/artifacts/{HELD}")),
    ] {
        let (s, body) = call(
            &client,
            method.clone(),
            format!("{base}{path}"),
            &other_token,
        )
        .await;
        assert!(
            matches!(s, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "{method} {path}: {s} {body}"
        );
    }
    let (_, links) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{t}/artifacts"),
        &token,
    )
    .await;
    assert_eq!(links.as_array().map(Vec::len), Some(1), "{links}");

    // The MCP tools say the same.
    let rpc = |name: &'static str, arguments: Value| {
        let (client, base, token) = (client.clone(), base.clone(), token.clone());
        async move {
            let resp: Value = client
                .post(format!("{base}/mcp"))
                .header("Authorization", &token)
                .json(&json!({
                    "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": name, "arguments": arguments}
                }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let text = resp["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_else(|| panic!("{name}: {resp}"))
                .to_string();
            serde_json::from_str::<Value>(&text).unwrap()
        }
    };
    let listed = rpc("list_thread_artifacts", json!({"thread_id": t})).await;
    assert_eq!(listed[0]["sha256"], HELD.to_ascii_lowercase());
    let removed = rpc(
        "unlink_thread_artifact",
        json!({"thread_id": t, "sha256": HELD}),
    )
    .await;
    assert_eq!(removed, json!({"removed": true}));
    let version = rpc("get_thread_version", json!({"thread_id": t})).await;
    assert_eq!(version["version"], 2, "the unlink moved it too");
    let (s, _) = call(
        &client,
        Method::DELETE,
        format!("{base}/threads/{t}/artifacts/{HELD}"),
        &token,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "nothing left to unlink");
}

#[tokio::test]
async fn a_hand_off_to_review_pins_what_was_handed_over() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let (ws, worker, token, thread) = tenant(store.as_ref(), "a").await;
    let (_, _, other_token, _) = tenant(store.as_ref(), "b").await;
    let t = thread.0;
    let held = HELD.to_ascii_lowercase();
    store.record_artifact_ref(ws, &held).await.unwrap();

    let (s, _) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{t}/review-packet"),
        &token,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "nothing handed over yet");

    store.claim_thread(thread, worker).await.unwrap();
    let result = json!({"status": "done"});
    store
        .set_thread_result(thread, worker, &result)
        .await
        .unwrap();
    store
        .link_thread_artifact(thread, &held, worker)
        .await
        .unwrap();
    store
        .transition_thread(thread, worker, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();

    let (s, packet) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{t}/review-packet"),
        &token,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{packet}");
    let version = store.thread_version(thread).await.unwrap();
    assert_eq!(packet["thread_version"], version);
    assert_eq!(packet["manifest"]["artifacts"], json!([held]));
    assert_eq!(
        packet["manifest"]["result"]["sha256"],
        maidan_types::result_sha256(&result).unwrap()
    );
    assert_eq!(packet["requested_by"], json!(worker.0));
    let manifest: maidan_types::EvidenceManifest =
        serde_json::from_value(packet["manifest"].clone()).unwrap();
    assert_eq!(packet["evidence_root"], manifest.root().unwrap());

    let (s, _) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{t}/review-packet"),
        &other_token,
    )
    .await;
    assert!(
        matches!(s, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "another workspace reads no packet: {s}"
    );

    let resp: Value = client
        .post(format!("{base}/mcp"))
        .header("Authorization", &token)
        .json(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "get_review_packet", "arguments": {"thread_id": t}}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let via_mcp: Value = serde_json::from_str(text).unwrap();
    assert_eq!(via_mcp["evidence_root"], packet["evidence_root"]);
}
