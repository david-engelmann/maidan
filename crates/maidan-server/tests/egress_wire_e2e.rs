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
use maidan_server::github::{GithubApiClient, GithubError, GithubIssueComment, GithubSender};
use maidan_server::slack::{SlackError, SlackSender, SlackWebClient};
use maidan_types::ExternalRef;
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
async fn slack_client_posts_chat_postmessage_and_returns_the_message_ref() {
    let (base, rec) = spawn(
        StatusCode::OK,
        json!({ "ok": true, "ts": "1699999999.001200" }),
    )
    .await;
    let client = SlackWebClient::with_base_url("xoxb-secret".into(), base);
    let reference = client
        .post_message("C123", "hello slack", None)
        .await
        .unwrap();

    assert_eq!(
        reference,
        Some(ExternalRef::Slack {
            channel_id: "C123".into(),
            ts: "1699999999.001200".into()
        }),
        "the ref is what makes a re-delivery an edit rather than a second message"
    );

    let reqs = rec.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/api/chat.postMessage");
    assert_eq!(r.auth, "Bearer xoxb-secret");
    assert_eq!(r.body["channel"], "C123");
    assert_eq!(r.body["text"], "hello slack");
    assert!(
        r.body.get("thread_ts").is_none(),
        "a top-level post omits thread_ts entirely — Slack rejects an explicit null"
    );
}

/// A post that Slack accepted but whose `ts` we cannot read is **not** a failed
/// delivery: the message exists, and reporting a failure would make the worker
/// retry and post a second copy. We simply have no ref to store.
#[tokio::test]
async fn slack_client_reports_a_ts_less_success_as_delivered_without_a_ref() {
    let (base, _rec) = spawn(StatusCode::OK, json!({ "ok": true })).await;
    let client = SlackWebClient::with_base_url("xoxb-secret".into(), base);
    assert_eq!(
        client.post_message("C123", "x", None).await.unwrap(),
        None,
        "posted, but not addressable"
    );
}

/// Cluster 378.2: a re-delivery can reply *inside* the Slack thread it first
/// posted in, rather than starting a new top-level message.
#[tokio::test]
async fn slack_client_threads_a_reply_under_a_parent_ts() {
    let (base, rec) = spawn(
        StatusCode::OK,
        json!({ "ok": true, "ts": "1699999999.002000" }),
    )
    .await;
    let client = SlackWebClient::with_base_url("xoxb-secret".into(), base);
    client
        .post_message("C123", "a follow-up", Some("1699999999.001200"))
        .await
        .unwrap();

    let reqs = rec.lock().unwrap();
    assert_eq!(reqs[0].body["thread_ts"], "1699999999.001200");
    assert_eq!(reqs[0].body["channel"], "C123");
}

/// `chat.update` edits in place. Addressed by channel **and** `ts` — the `ts`
/// alone does not identify a Slack message.
#[tokio::test]
async fn slack_client_updates_a_message_via_chat_update() {
    let (base, rec) = spawn(
        StatusCode::OK,
        json!({ "ok": true, "ts": "1699999999.001200" }),
    )
    .await;
    let client = SlackWebClient::with_base_url("xoxb-secret".into(), base);
    client
        .update_message("C123", "1699999999.001200", "the revised review")
        .await
        .unwrap();

    let reqs = rec.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/api/chat.update");
    assert_eq!(r.auth, "Bearer xoxb-secret");
    assert_eq!(r.body["channel"], "C123");
    assert_eq!(r.body["ts"], "1699999999.001200");
    assert_eq!(r.body["text"], "the revised review");
}

/// An update against a message that is gone is a config-class error, so it
/// disables the link exactly as a failed post would (Cluster 377.3).
#[tokio::test]
async fn slack_client_maps_an_update_error_to_a_misconfiguration() {
    let (base, _rec) = spawn(
        StatusCode::OK,
        json!({ "ok": false, "error": "channel_not_found" }),
    )
    .await;
    let client = SlackWebClient::with_base_url("xoxb-secret".into(), base);
    let err = client
        .update_message("C404", "1699999999.001200", "x")
        .await
        .unwrap_err();
    assert!(err.is_misconfiguration(), "got {err:?}");
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
    let err = client.post_message("C404", "x", None).await.unwrap_err();
    match err {
        SlackError::Api(msg) => assert_eq!(msg, "channel_not_found"),
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn github_client_posts_issue_comment_and_returns_the_comment_ref() {
    let (base, rec) = spawn(StatusCode::CREATED, json!({ "id": 998877 })).await;
    let client = GithubApiClient::with_base_url("ghp-secret".into(), base);
    let reference = client
        .post_comment("acme/widgets", 42, "hello github")
        .await
        .unwrap();

    assert_eq!(
        reference,
        Some(ExternalRef::Github {
            repo: "acme/widgets".into(),
            comment_id: 998877
        })
    );

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

/// The comment was created, so an unreadable `id` must not be reported as a
/// failure — a retry would leave two comments on the PR. The recovery path for a
/// lost ref is the hidden marker in the body (Cluster 379.4), never a re-post.
#[tokio::test]
async fn github_client_reports_an_idless_success_as_delivered_without_a_ref() {
    for response in [json!({}), json!({ "id": 0 }), json!({ "id": "998877" })] {
        let (base, _rec) = spawn(StatusCode::CREATED, response.clone()).await;
        let client = GithubApiClient::with_base_url("ghp-secret".into(), base);
        assert_eq!(
            client
                .post_comment("acme/widgets", 42, "x")
                .await
                .expect("a created comment is never a failure"),
            None,
            "posted, but not addressable ({response})"
        );
    }
}

/// `PATCH /repos/{repo}/issues/comments/{id}` — addressed by repository and
/// comment id, with **no issue number in the path**. That is why `ExternalRef`
/// does not carry one.
#[tokio::test]
async fn github_client_updates_a_comment_by_id_without_the_issue_number() {
    let (base, rec) = spawn(StatusCode::OK, json!({ "id": 998877 })).await;
    let client = GithubApiClient::with_base_url("ghp-secret".into(), base);
    client
        .update_comment("acme/widgets", 998877, "the revised review")
        .await
        .unwrap();

    let reqs = rec.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "PATCH");
    assert_eq!(r.path, "/repos/acme/widgets/issues/comments/998877");
    assert_eq!(r.auth, "Bearer ghp-secret");
    assert_eq!(r.user_agent, "maidan-projector");
    assert_eq!(r.body["body"], "the revised review");
}

/// A deleted comment answers 404, which is a misconfiguration — the same
/// classification a failed post gets (Cluster 377.3), so an update cannot retry
/// forever against something that no longer exists.
#[tokio::test]
async fn github_client_maps_an_update_404_to_a_misconfiguration() {
    let (base, _rec) = spawn(StatusCode::NOT_FOUND, json!({ "message": "Not Found" })).await;
    let client = GithubApiClient::with_base_url("ghp-secret".into(), base);
    let err = client
        .update_comment("acme/widgets", 1, "x")
        .await
        .unwrap_err();
    assert!(err.is_misconfiguration(), "got {err:?}");
}

/// A rate-limited update is a 403 too, and must **not** disable the link.
#[tokio::test]
async fn github_client_does_not_treat_a_rate_limited_update_as_a_misconfiguration() {
    let (base, _rec) = spawn_with_headers(
        StatusCode::FORBIDDEN,
        json!({ "message": "API rate limit exceeded" }),
        &[("retry-after", "60")],
    )
    .await;
    let client = GithubApiClient::with_base_url("ghp-secret".into(), base);
    let err = client
        .update_comment("acme/widgets", 1, "x")
        .await
        .unwrap_err();
    assert!(!err.is_misconfiguration(), "got {err:?}");
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

/// `GET /repos/{repo}/issues/{n}/comments` — the Cluster 379.4 recovery scan.
#[tokio::test]
async fn github_client_lists_issue_comments() {
    let (base, rec) = spawn(
        StatusCode::OK,
        json!([{ "id": 11, "body": "<!-- maidan:result:x -->\nreview" }]),
    )
    .await;
    let client = GithubApiClient::with_base_url("ghp-secret".into(), base);
    let comments = client
        .list_issue_comments("acme/widgets", 42)
        .await
        .unwrap();
    assert_eq!(
        comments,
        vec![GithubIssueComment {
            id: 11,
            body: "<!-- maidan:result:x -->\nreview".into(),
        }]
    );
    let reqs = rec.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "GET");
    assert_eq!(r.path, "/repos/acme/widgets/issues/42/comments");
    assert_eq!(r.auth, "Bearer ghp-secret");
    assert_eq!(r.user_agent, "maidan-projector");
}
