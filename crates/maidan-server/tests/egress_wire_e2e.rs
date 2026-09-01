//! Cluster 347: real-client wire-path tests for the projector egress. The
//! `SlackWebClient` / `GithubApiClient` (the production HTTP clients that build the
//! actual outbound request) were never exercised — the projector egress tests use
//! mock `SlackSender`/`GithubSender` traits. These point the real clients at a
//! loopback server (via the new `with_base_url`) and assert the exact request they
//! send, plus the success/error decoding.

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
    routing::any,
    Json, Router,
};
use maidan_server::github::{GithubApiClient, GithubError, GithubSender};
use maidan_server::slack::{SlackError, SlackSender, SlackWebClient};
use serde_json::{json, Value};

#[derive(Clone)]
struct Recorded {
    method: String,
    path: String,
    auth: String,
    user_agent: String,
    body: Value,
}

#[derive(Clone)]
struct TestSrv {
    rec: Arc<Mutex<Vec<Recorded>>>,
    status: StatusCode,
    response: Value,
}

async fn handler(
    State(srv): State<TestSrv>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let get = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    srv.rec.lock().unwrap().push(Recorded {
        method: method.to_string(),
        path: uri.path().to_string(),
        auth: get("authorization"),
        user_agent: get("user-agent"),
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    (srv.status, Json(srv.response.clone())).into_response()
}

/// Spawn a loopback server that records every request and answers with
/// `(status, response)`. Returns its base URL and the shared recorder.
async fn spawn(status: StatusCode, response: Value) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    let rec = Arc::new(Mutex::new(Vec::new()));
    let srv = TestSrv {
        rec: rec.clone(),
        status,
        response,
    };
    let app = Router::new().fallback(any(handler)).with_state(srv);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), rec)
}

#[tokio::test]
async fn slack_client_posts_chat_postmessage_and_decodes_ok() {
    let (base, rec) = spawn(StatusCode::OK, json!({ "ok": true })).await;
    let client = SlackWebClient::with_base_url("xoxb-secret".into(), base);
    client.post_message("C123", "hello slack").await.unwrap();

    let reqs = rec.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/api/chat.postMessage");
    assert_eq!(r.auth, "Bearer xoxb-secret");
    assert_eq!(r.body["channel"], "C123");
    assert_eq!(r.body["text"], "hello slack");
}

#[tokio::test]
async fn slack_client_maps_ok_false_to_api_error() {
    // Slack returns HTTP 200 with `{"ok": false, "error": ...}` on logical errors.
    let (base, _rec) = spawn(
        StatusCode::OK,
        json!({ "ok": false, "error": "channel_not_found" }),
    )
    .await;
    let client = SlackWebClient::with_base_url("xoxb-secret".into(), base);
    let err = client.post_message("C404", "x").await.unwrap_err();
    match err {
        SlackError::Api(msg) => assert_eq!(msg, "channel_not_found"),
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn github_client_posts_issue_comment_with_required_headers() {
    let (base, rec) = spawn(StatusCode::CREATED, json!({ "id": 1 })).await;
    let client = GithubApiClient::with_base_url("ghp-secret".into(), base);
    client
        .post_comment("acme/widgets", 42, "hello github")
        .await
        .unwrap();

    let reqs = rec.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/repos/acme/widgets/issues/42/comments");
    assert_eq!(r.auth, "Bearer ghp-secret");
    // GitHub rejects requests without a User-Agent — the client must set one.
    assert_eq!(r.user_agent, "maidan-projector");
    assert_eq!(r.body["body"], "hello github");
}

#[tokio::test]
async fn github_client_maps_non_success_to_api_error() {
    let (base, _rec) = spawn(StatusCode::NOT_FOUND, json!({ "message": "Not Found" })).await;
    let client = GithubApiClient::with_base_url("ghp-secret".into(), base);
    let err = client
        .post_comment("acme/missing", 1, "x")
        .await
        .unwrap_err();
    match err {
        GithubError::Api(code) => assert_eq!(code, 404),
        other => panic!("expected Api error, got {other:?}"),
    }
}
