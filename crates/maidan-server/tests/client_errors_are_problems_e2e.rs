//! Every client error the HTTP API answers is an RFC 9457 problem: a malformed
//! path parameter, query string or JSON body, a body not sent as JSON, one over
//! the body-size limit, an unknown route and a wrong method, on real routes.
//! SCIM and A2A answer the same rejections in their own error envelopes.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::{header, header::HeaderMap, RequestBuilder, StatusCode};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

/// The body-size cap every test here runs under.
const MAX_BODY_BYTES: &str = "1024";
const ID: &str = "00000000-0000-0000-0000-000000000001";

struct Server {
    base: String,
    client: reqwest::Client,
    _artifacts: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

/// Auth is disabled so a request reaches the extractors (auth would 401 first).
async fn spawn() -> Server {
    // `router` reads the cap once, at build; every test sets the same value.
    std::env::set_var("MAIDAN_MAX_BODY_BYTES", MAX_BODY_BYTES);
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let artifacts = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(artifacts.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        _artifacts: artifacts,
        task,
    }
}

/// Send `request` and require an `application/problem+json` answer with
/// `status`, whose body repeats the status and says what went wrong.
async fn assert_problem(request: RequestBuilder, status: StatusCode) -> HeaderMap {
    let response = request.send().await.unwrap();
    let url = response.url().clone();
    let headers = response.headers().clone();
    assert_eq!(response.status(), status, "{url}");
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json"),
        "{url}"
    );
    let problem: Value = response.json().await.unwrap();
    assert_eq!(problem["status"], status.as_u16(), "{url}: {problem}");
    assert!(
        problem["type"]
            .as_str()
            .is_some_and(|t| t.starts_with("https://maidan.dev/problems/")),
        "{url}: {problem}"
    );
    assert!(
        problem["detail"].as_str().is_some_and(|d| !d.is_empty()),
        "{url}: {problem}"
    );
    headers
}

#[tokio::test]
async fn a_malformed_path_parameter_is_a_400_problem() {
    let s = spawn().await;
    assert_problem(
        s.client.get(s.url("/threads/not-a-uuid")),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_problem(
        s.client
            .get(s.url(&format!("/workspaces/not-a-uuid/dm?member_id={ID}"))),
        StatusCode::BAD_REQUEST,
    )
    .await;
}

#[tokio::test]
async fn a_malformed_query_string_is_a_400_problem() {
    let s = spawn().await;
    assert_problem(
        s.client
            .get(s.url(&format!("/channels/{ID}/threads?limit=many"))),
        StatusCode::BAD_REQUEST,
    )
    .await;
    // `member_id` is required on the direct-message list.
    assert_problem(
        s.client.get(s.url(&format!("/workspaces/{ID}/dm"))),
        StatusCode::BAD_REQUEST,
    )
    .await;
}

#[tokio::test]
async fn a_json_body_that_does_not_parse_or_fit_is_a_400_problem() {
    let s = spawn().await;
    for body in ["{not json", r#"{"name": 5}"#, "[]"] {
        assert_problem(
            s.client
                .post(s.url("/workspaces"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(body),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    // The direct-message routes took axum's plain `Json` and answered in text.
    assert_problem(
        s.client
            .post(s.url(&format!("/workspaces/{ID}/dm")))
            .header(header::CONTENT_TYPE, "application/json")
            .body("{not json"),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_problem(
        s.client
            .post(s.url(&format!("/group-dms/{ID}/messages")))
            .header(header::CONTENT_TYPE, "application/json")
            .body(r#"{"body": ["not", "text"]}"#),
        StatusCode::BAD_REQUEST,
    )
    .await;
}

#[tokio::test]
async fn a_body_not_sent_as_json_is_a_415_problem() {
    let s = spawn().await;
    assert_problem(
        s.client.post(s.url("/workspaces")).body(r#"{"name":"w"}"#),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
    )
    .await;
    assert_problem(
        s.client
            .post(s.url(&format!("/dm/{ID}/messages")))
            .header(header::CONTENT_TYPE, "text/plain")
            .body(r#"{"body":"hi"}"#),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
    )
    .await;
}

#[tokio::test]
async fn a_body_over_the_limit_is_a_413_problem() {
    let s = spawn().await;
    let big = "x".repeat(4096);
    assert_problem(
        s.client
            .post(s.url("/workspaces"))
            .json(&json!({ "name": big })),
        StatusCode::PAYLOAD_TOO_LARGE,
    )
    .await;
    // A raw upload is capped the same way.
    assert_problem(
        s.client
            .post(s.url("/artifacts?kind=attachment&mime_type=text/plain"))
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(big),
        StatusCode::PAYLOAD_TOO_LARGE,
    )
    .await;
}

#[tokio::test]
async fn an_unknown_route_is_a_404_and_a_wrong_method_a_405_problem() {
    let s = spawn().await;
    assert_problem(s.client.get(s.url("/no/such/route")), StatusCode::NOT_FOUND).await;
    let wrong_method = assert_problem(
        s.client.delete(s.url("/workspaces")),
        StatusCode::METHOD_NOT_ALLOWED,
    )
    .await;
    let allow = wrong_method
        .get(header::ALLOW)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(allow.contains("POST"), "Allow: {allow:?}");
}

#[tokio::test]
async fn a_well_formed_request_still_succeeds() {
    let s = spawn().await;
    let created = s
        .client
        .post(s.url("/workspaces"))
        .json(&json!({ "name": "fine" }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn scim_answers_a_malformed_id_in_its_own_envelope() {
    let s = spawn().await;
    let response = s
        .client
        .get(s.url("/scim/v2/Users/not-a-uuid"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/scim+json")
    );
    let error: Value = response.json().await.unwrap();
    assert_eq!(
        error["schemas"][0],
        "urn:ietf:params:scim:api:messages:2.0:Error"
    );
    assert_eq!(error["status"], "400");
}

#[tokio::test]
async fn a2a_answers_an_unreadable_request_in_json_rpc_and_its_rest_binding() {
    let s = spawn().await;
    let rpc: Value = s
        .client
        .post(s.url("/a2a/v1/rpc"))
        .header(header::CONTENT_TYPE, "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rpc["jsonrpc"], "2.0");
    assert_eq!(rpc["id"], Value::Null);
    assert_eq!(rpc["error"]["code"], -32700);

    let invalid: Value = s
        .client
        .post(s.url("/a2a/v1/rpc"))
        .json(&json!({ "not": "a request" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invalid["error"]["code"], -32600);

    let rest = s
        .client
        .post(s.url("/a2a/v1/message:send"))
        .header(header::CONTENT_TYPE, "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(rest.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        rest.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    let body: Value = rest.json().await.unwrap();
    assert_eq!(body["error"]["code"], 400);
    assert_eq!(body["error"]["status"], "INVALID_ARGUMENT");
}
