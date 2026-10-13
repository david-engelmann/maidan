//! The production `LinearClient` on the wire, against a loopback stand-in
//! for Linear's GraphQL endpoint: the key goes in the header the way Linear
//! reads it, each operation sends the variables Linear expects, and Linear's
//! answers (data, an `errors` array on HTTP 200) decode to what the worker
//! acts on.

// The stand-in is a plain axum server, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::sync::{Arc, Mutex};

use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use maidan_server::linear::{LinearClient, LinearError, LinearSender};
use serde_json::{json, Value};

type Seen = Arc<Mutex<Vec<(Option<String>, Value)>>>;

/// Answer by operation name, and record each request's Authorization header
/// and body.
async fn graphql(
    State(seen): State<Seen>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    seen.lock().unwrap().push((auth, body.clone()));
    let query = body["query"].as_str().unwrap_or_default();
    let reply = if query.contains("MaidanIssueState") {
        json!({"errors": [{"message": "Authentication required", "extensions": {"code": "AUTHENTICATION_ERROR"}}]})
    } else if query.contains("MaidanIssue") {
        if body["variables"]["id"] == "ENG-404" {
            json!({"data": {"issue": null}, "errors": [{"message": "Entity not found: Issue", "extensions": {"code": "INVALID_INPUT"}}]})
        } else {
            json!({"data": {"issue": {
                "id": "11111111-2222-4333-8444-555555555555",
                "identifier": "ENG-7",
                "team": {"id": "6a1f1c4e-2f0b-4d5e-9a7b-0c1d2e3f4a5b", "states": {"nodes": [
                    {"id": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee", "name": "In Review"}
                ]}}
            }}})
        }
    } else if query.contains("MaidanCommentUpdate") {
        json!({"data": {"commentUpdate": {"success": true}}})
    } else if query.contains("MaidanComment") {
        json!({"data": {"commentCreate": {"success": true, "comment": {"id": "cccccccc-0000-4000-8000-000000000001"}}}})
    } else {
        json!({"errors": [{"message": "unknown operation"}]})
    };
    Json(reply)
}

async fn spawn() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route("/graphql", post(graphql))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/graphql"), seen)
}

#[tokio::test]
async fn the_client_reads_an_issue_comments_edits_and_reports_errors_as_linear_sends_them() {
    let (url, seen) = spawn().await;
    let client = LinearClient::with_url("lin_api_test_key".into(), url);

    let issue = client.issue("ENG-7").await.unwrap();
    assert_eq!(issue.id, "11111111-2222-4333-8444-555555555555");
    assert_eq!(issue.team_id, "6a1f1c4e-2f0b-4d5e-9a7b-0c1d2e3f4a5b");
    assert_eq!(
        issue.state_named("In Review").map(|s| s.id.as_str()),
        Some("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")
    );

    let comment = client.create_comment(&issue.id, "hello").await.unwrap();
    assert_eq!(comment, "cccccccc-0000-4000-8000-000000000001");
    client
        .update_comment(&comment, "hello again")
        .await
        .unwrap();

    let missing = client.issue("ENG-404").await.unwrap_err();
    assert!(matches!(missing, LinearError::NotFound(_)), "{missing}");
    let denied = client
        .move_issue(&issue.id, "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")
        .await
        .unwrap_err();
    assert!(denied.is_misconfiguration(), "{denied}");

    let seen = seen.lock().unwrap();
    // A personal key goes bare, the way Linear reads one.
    assert!(seen
        .iter()
        .all(|(auth, _)| auth.as_deref() == Some("lin_api_test_key")));
    let create = &seen[1].1;
    assert_eq!(
        create["variables"]["input"],
        json!({"issueId": "11111111-2222-4333-8444-555555555555", "body": "hello"})
    );
    let update = &seen[2].1;
    assert_eq!(
        update["variables"]["id"],
        "cccccccc-0000-4000-8000-000000000001"
    );
    assert_eq!(update["variables"]["input"], json!({"body": "hello again"}));
}

#[tokio::test]
async fn an_oauth_token_is_sent_as_a_bearer() {
    let (url, seen) = spawn().await;
    let client = LinearClient::with_url("oauth-access-token".into(), url);
    client.issue("ENG-7").await.unwrap();
    assert_eq!(
        seen.lock().unwrap()[0].0.as_deref(),
        Some("Bearer oauth-access-token")
    );
}
