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
    /// Response headers to echo back — how GitHub distinguishes a rate-limited
    /// 403 from a permission-denied one (Cluster 377.3).
    response_headers: Vec<(String, String)>,
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
    let mut response = (srv.status, Json(srv.response.clone())).into_response();
    for (name, value) in &srv.response_headers {
        response.headers_mut().insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    response
}

/// Spawn a loopback server that records every request and answers with
/// `(status, response)`. Returns its base URL and the shared recorder.
async fn spawn(status: StatusCode, response: Value) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    spawn_with_headers(status, response, &[]).await
}

/// [`spawn`], plus response headers.
async fn spawn_with_headers(
    status: StatusCode,
    response: Value,
    response_headers: &[(&str, &str)],
) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    let rec = Arc::new(Mutex::new(Vec::new()));
    let srv = TestSrv {
        rec: rec.clone(),
        status,
        response,
        response_headers: response_headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
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
        GithubError::Api {
            status,
            rate_limited,
        } => {
            assert_eq!(status, 404);
            assert!(!rate_limited, "a plain 404 carries no rate-limit headers");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

/// GitHub answers a secondary rate limit with **403** — the same status as a
/// revoked token — so the classification that decides whether to disable a link
/// (Cluster 377.3) has to read the headers, not the status.
#[tokio::test]
async fn github_client_marks_a_rate_limited_403_as_rate_limited() {
    let (base, _rec) = spawn_with_headers(
        StatusCode::FORBIDDEN,
        json!({ "message": "API rate limit exceeded" }),
        &[("x-ratelimit-remaining", "0")],
    )
    .await;
    let client = GithubApiClient::with_base_url("ghp-secret".into(), base);
    let err = client
        .post_comment("acme/widgets", 1, "x")
        .await
        .unwrap_err();
    match err {
        GithubError::Api {
            status,
            rate_limited,
        } => {
            assert_eq!(status, 403);
            assert!(rate_limited);
            assert!(
                !err.is_misconfiguration(),
                "a rate limit must not disable the link"
            );
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}
