//! Review accounts (`POST /operator/review-accounts`).
//!
//! A directory or a health check that reviews the hosted service cannot answer
//! a sign-up's emailed code, so the operator mints it a tenant instead: the
//! demo sandbox's workspace, named for the provider, with tokens that expire
//! after `ttl_days`. Only an operator may ask, the account is never served by
//! the instance's own credentials, and one provider's account is another
//! tenant to every other.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::capability;
use maidan_bus::InMemoryBus;
use maidan_server::{
    router,
    sandbox::SandboxSettings,
    slack::{SlackError, SlackSender},
    test_support::{tenant, token},
    AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{ExternalRef, WorkspaceId, WorkspaceOrigin};
use reqwest::StatusCode;
use serde_json::{json, Value};

const ORIGIN: &str = "https://maidan.example";

struct Server {
    base: String,
    store: Arc<dyn Store>,
    http: reqwest::Client,
}

async fn spawn(adjust: impl FnOnce(&mut AppState)) -> Server {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::for_tests(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(16)),
        search,
    );
    std::mem::forget(dir);
    state.public_origin = Some(ORIGIN.into());
    state.federation = FederationRuntime::new(true, Some(Arc::new([9u8; 32])));
    adjust(&mut state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Server {
        base: format!("http://{addr}"),
        store,
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
    }
}

impl Server {
    async fn mint(&self, bearer: &str, body: Value) -> reqwest::Response {
        self.http
            .post(format!("{}/operator/review-accounts", self.base))
            .bearer_auth(bearer)
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    /// A review account the operator minted, its answer.
    async fn minted(&self, operator: &str, body: Value) -> Value {
        let res = self.mint(operator, body).await;
        let status = res.status();
        let text = res.text().await.unwrap();
        assert_eq!(status, StatusCode::CREATED, "{text}");
        serde_json::from_str(&text).unwrap()
    }

    async fn get(&self, bearer: &str, path: &str) -> (StatusCode, Value) {
        let res = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap();
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn workspaces(&self) -> i64 {
        self.store.count_workspaces().await.unwrap()
    }
}

fn s<'a>(v: &'a Value, path: &[&str]) -> &'a str {
    let mut at = v;
    for key in path {
        at = &at[*key];
    }
    at.as_str()
        .unwrap_or_else(|| panic!("{path:?} is not a string in {v}"))
}

fn time(v: &Value, path: &[&str]) -> DateTime<Utc> {
    s(v, path).parse().unwrap()
}

fn workspace_id(minted: &Value) -> WorkspaceId {
    WorkspaceId(s(minted, &["workspace_id"]).parse().unwrap())
}

#[tokio::test]
async fn an_operator_mints_a_review_account_whose_three_tokens_work() {
    let server = spawn(|_| {}).await;
    let operator = tenant(server.store.as_ref(), "operator").await;
    let res = server
        .mint(
            &operator.token,
            json!({ "provider": "glama", "ttl_days": 30 }),
        )
        .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(
        res.headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store"),
        "the answer holds three secrets"
    );
    let minted: Value = res.json().await.unwrap();

    assert_eq!(s(&minted, &["provider"]), "glama");
    assert_eq!(
        s(&minted, &["mcp_url"]),
        "https://maidan.example/mcp/streamable"
    );
    assert_eq!(s(&minted, &["console_url"]), "https://maidan.example/ui/");
    let expires = time(&minted, &["expires_at"]);
    let minutes = (expires - Utc::now()).num_minutes();
    let thirty_days = 30 * 24 * 60;
    assert!(
        (thirty_days - 5..=thirty_days).contains(&minutes),
        "the tokens live ttl_days, not {minutes} minutes"
    );

    let ws = workspace_id(&minted);
    let ws_text = ws.0.to_string();
    let mut secrets = Vec::new();
    for (who, label) in [
        ("admin", "review glama admin"),
        ("agent", "review glama agent"),
        ("reviewer", "review glama reviewer"),
    ] {
        let secret = s(&minted, &[who, "token"]);
        secrets.push(secret);
        let (status, me) = server.get(secret, "/me").await;
        assert_eq!(status, StatusCode::OK, "{who}: {me}");
        assert_eq!(s(&me, &["workspace_id"]), ws_text, "{who}");
        assert_eq!(
            s(&me, &["member_id"]),
            s(&minted, &[who, "member_id"]),
            "{who}"
        );

        // The token expires with the account, under the label that names it,
        // and the admin's listing never shows the secret itself.
        let member = s(&minted, &[who, "member_id"]);
        let (status, listed) = server
            .get(
                s(&minted, &["admin", "token"]),
                &format!("/workspaces/{ws_text}/members/{member}/tokens"),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{who}: {listed}");
        let rows = listed.as_array().unwrap();
        let row = rows
            .iter()
            .find(|row| row["label"] == json!(label))
            .unwrap_or_else(|| panic!("{who} has no token labelled {label}: {listed}"));
        assert!(!listed.to_string().contains(secret), "{who}");
        let lapses = time(row, &["expires_at"]);
        assert!((lapses - expires).num_seconds().abs() <= 1, "{who}");
    }
    assert_ne!(secrets[0], secrets[1]);
    assert_ne!(secrets[1], secrets[2]);
    assert_ne!(
        s(&minted, &["agent", "member_id"]),
        s(&minted, &["reviewer", "member_id"])
    );

    // An origin of `open` puts it under the free allowance and keeps the
    // instance's credentials from serving it.
    assert_eq!(
        server.store.workspace_origin(ws).await.unwrap(),
        WorkspaceOrigin::Open
    );
    let admin = s(&minted, &["admin", "token"]);
    let (status, workspace) = server.get(admin, &format!("/workspaces/{ws_text}")).await;
    assert_eq!(status, StatusCode::OK, "{workspace}");
    assert_eq!(s(&workspace, &["name"]), "Review: glama");

    let (status, channels) = server
        .get(admin, &format!("/workspaces/{ws_text}/channels"))
        .await;
    assert_eq!(status, StatusCode::OK, "{channels}");
    let channel = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|ch| ch["id"] == minted["channel_id"])
        .unwrap_or_else(|| panic!("no #general in {channels}"));
    assert_eq!(channel["name"], json!("general"));
    assert_eq!(
        channel["topic"],
        json!("Sample work for glama's checks. Nothing here belongs to anyone.")
    );

    let task = s(&minted, &["task_id"]);
    let (status, thread) = server.get(admin, &format!("/threads/{task}")).await;
    assert_eq!(status, StatusCode::OK, "{thread}");
    assert_eq!(thread["title"], json!("Say hello"), "{thread}");
    let (status, messages) = server
        .get(admin, &format!("/threads/{task}/messages"))
        .await;
    assert_eq!(status, StatusCode::OK, "{messages}");
    let messages = messages.as_array().unwrap();
    assert_eq!(messages.len(), 1, "one opening message: {messages:?}");
    assert!(
        s(&messages[0], &["body"]).starts_with("Welcome to Maidan."),
        "{messages:?}"
    );
}

#[tokio::test]
async fn a_review_account_is_audited_in_its_own_workspace() {
    let server = spawn(|_| {}).await;
    let operator = tenant(server.store.as_ref(), "operator").await;
    let minted = server
        .minted(
            &operator.token,
            json!({ "provider": "cursor", "ttl_days": 7 }),
        )
        .await;
    let ws = s(&minted, &["workspace_id"]);
    let (status, audit) = server
        .get(
            s(&minted, &["admin", "token"]),
            &format!("/workspaces/{ws}/audit?limit=100"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{audit}");
    let rows = audit.as_array().unwrap();

    let created: Vec<&Value> = rows
        .iter()
        .filter(|row| row["action"] == json!("review_account.create"))
        .collect();
    assert_eq!(created.len(), 1, "one create row: {audit}");
    assert_eq!(
        created[0]["actor_id"],
        json!(operator.admin_id.0.to_string()),
        "it names the operator who asked"
    );
    assert_eq!(created[0]["metadata"]["provider"], json!("cursor"));
    assert_eq!(created[0]["metadata"]["expires_at"], minted["expires_at"]);

    let mints: Vec<&Value> = rows
        .iter()
        .filter(|row| row["action"] == json!("token.mint"))
        .collect();
    assert_eq!(mints.len(), 3, "a mint row for each token: {audit}");
    assert!(
        mints
            .iter()
            .all(|row| row["metadata"]["source"] == json!("review-account")),
        "{audit}"
    );
}

#[tokio::test]
async fn a_workspace_admin_without_operator_global_is_refused_and_nothing_is_made() {
    let server = spawn(|_| {}).await;
    let other = tenant(server.store.as_ref(), "someone else").await;
    let everything_but_operator: Vec<String> = capability::all()
        .into_iter()
        .filter(|cap| cap != capability::OPERATOR_GLOBAL)
        .collect();
    let admin = token(
        server.store.as_ref(),
        other.workspace_id,
        other.admin_id,
        everything_but_operator,
    )
    .await;
    let before = server.workspaces().await;

    let res = server
        .mint(&admin, json!({ "provider": "glama", "ttl_days": 30 }))
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let body = res.text().await.unwrap();
    assert!(body.contains("operator:global"), "{body}");

    let anonymous = server
        .http
        .post(format!("{}/operator/review-accounts", server.base))
        .json(&json!({ "provider": "glama" }))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(server.workspaces().await, before, "nothing was created");
}

#[tokio::test]
async fn a_bad_provider_or_lifetime_is_a_400_that_creates_nothing() {
    let server = spawn(|_| {}).await;
    let operator = tenant(server.store.as_ref(), "operator").await;
    let before = server.workspaces().await;

    let too_long = "a".repeat(33);
    for provider in [
        "Glama!",
        "",
        too_long.as_str(),
        "glama corp",
        " glama",
        "glama_2",
    ] {
        let res = server
            .mint(&operator.token, json!({ "provider": provider }))
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{provider:?}");
        let body = res.text().await.unwrap();
        assert!(body.contains("a-z, 0-9 and -"), "{provider:?}: {body}");
    }
    let missing = server.mint(&operator.token, json!({})).await;
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST, "no provider");
    for ttl_days in [0, 91, -1] {
        let res = server
            .mint(
                &operator.token,
                json!({ "provider": "glama", "ttl_days": ttl_days }),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "ttl_days {ttl_days}");
        let body = res.text().await.unwrap();
        assert!(body.contains("ttl_days"), "{ttl_days}: {body}");
    }
    // A misspelled field is not read as a default.
    let typo = server
        .mint(
            &operator.token,
            json!({ "provider": "glama", "ttl_day": 1 }),
        )
        .await;
    assert_eq!(typo.status(), StatusCode::BAD_REQUEST, "an unknown field");

    assert_eq!(server.workspaces().await, before, "nothing was created");
}

#[tokio::test]
async fn the_lifetime_defaults_to_ninety_days_and_one_to_ninety_are_allowed() {
    let server = spawn(|_| {}).await;
    let operator = tenant(server.store.as_ref(), "operator").await;
    for (provider, body, days) in [
        ("defaulted", json!({ "provider": "defaulted" }), 90),
        (
            "longest",
            json!({ "provider": "longest", "ttl_days": 90 }),
            90,
        ),
        (
            "shortest",
            json!({ "provider": "shortest", "ttl_days": 1 }),
            1,
        ),
    ] {
        let minted = server.minted(&operator.token, body).await;
        let minutes = (time(&minted, &["expires_at"]) - Utc::now()).num_minutes();
        let want = days * 24 * 60;
        assert!(
            (want - 5..=want).contains(&minutes),
            "{provider}: {minutes} minutes, not {days} days"
        );
    }
}

#[tokio::test]
async fn without_a_public_origin_the_answer_is_a_503_that_names_the_setting() {
    let server = spawn(|state| state.public_origin = None).await;
    let operator = tenant(server.store.as_ref(), "operator").await;
    let before = server.workspaces().await;

    let res = server
        .mint(&operator.token, json!({ "provider": "glama" }))
        .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = res.text().await.unwrap();
    assert!(body.contains("MAIDAN_PUBLIC_ORIGIN"), "{body}");
    assert_eq!(server.workspaces().await, before, "nothing was created");
}

#[tokio::test]
async fn each_provider_gets_its_own_account_that_cannot_see_another() {
    let server = spawn(|_| {}).await;
    let operator = tenant(server.store.as_ref(), "operator").await;
    let first = server
        .minted(&operator.token, json!({ "provider": "glama" }))
        .await;
    let second = server
        .minted(&operator.token, json!({ "provider": "docker" }))
        .await;
    assert_ne!(first["workspace_id"], second["workspace_id"]);

    let second_ws = s(&second, &["workspace_id"]);
    for who in ["admin", "agent", "reviewer"] {
        let (status, body) = server
            .get(
                s(&first, &[who, "token"]),
                &format!("/workspaces/{second_ws}/channels"),
            )
            .await;
        assert!(
            matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "{who} of one account read another's channels: {status} {body}"
        );
    }
    let (_, workspace) = server
        .get(
            s(&second, &["admin", "token"]),
            &format!("/workspaces/{second_ws}"),
        )
        .await;
    assert_eq!(s(&workspace, &["name"]), "Review: docker");
}

#[tokio::test]
async fn review_accounts_need_no_demo_switch_and_ignore_the_sandbox_limit() {
    // With the sandbox off, its route is not there and the review one is.
    let off = spawn(|_| {}).await;
    let operator = tenant(off.store.as_ref(), "operator").await;
    let sandbox = off
        .http
        .post(format!("{}/sandbox", off.base))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(sandbox.status(), StatusCode::NOT_FOUND);
    off.minted(&operator.token, json!({ "provider": "glama" }))
        .await;

    // With the sandbox on and full (the operator's workspace is the one it
    // allows), the sandbox refuses and the review account does not notice.
    let full = spawn(|state| {
        state.sandbox = Some(Arc::new(SandboxSettings {
            max_sandboxes: 1,
            mcp_url: "https://try.example/mcp/streamable".into(),
            console_url: "https://try.example/ui/".into(),
        }));
    })
    .await;
    let operator = tenant(full.store.as_ref(), "operator").await;
    let refused = full
        .http
        .post(format!("{}/sandbox", full.base))
        .header("x-forwarded-for", "10.7.0.1")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    full.minted(&operator.token, json!({ "provider": "glama" }))
        .await;
}

/// A Slack the instance holds, so a workspace it serves shows it. Only the
/// listing reads it; nothing here sends.
struct InstanceSlack;

#[async_trait::async_trait]
impl SlackSender for InstanceSlack {
    async fn post_message(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError> {
        Err(SlackError::Http("these tests send nothing".into()))
    }

    async fn update_message(&self, _: &str, _: &str, _: &str) -> Result<(), SlackError> {
        Err(SlackError::Http("these tests send nothing".into()))
    }
}

/// The instance holds a Slack credential, and a workspace its operator opens
/// is served by it. A review account is opened by the operator too, but as
/// `open`, which `instance_credentials::serves` never lends the instance's
/// credential to: its reviewer sees no connection on any surface.
#[tokio::test]
async fn a_review_account_is_not_served_by_the_instances_credentials() {
    let server = spawn(|state| state.attach_slack_sender(Arc::new(InstanceSlack))).await;
    let operator = tenant(server.store.as_ref(), "operator").await;

    // The control: a workspace the operator opens shows the instance's Slack,
    // so the empty listing below is the rule at work and not an absent credential.
    let opened = server
        .http
        .post(format!("{}/operator/workspaces", server.base))
        .bearer_auth(&operator.token)
        .json(&json!({ "name": "opened", "admin_handle": "ada" }))
        .send()
        .await
        .unwrap();
    assert_eq!(opened.status(), StatusCode::CREATED);
    let opened: Value = opened.json().await.unwrap();
    let (status, rows) = server
        .get(
            s(&opened, &["token", "secret"]),
            &format!(
                "/workspaces/{}/connections",
                s(&opened, &["workspace", "id"])
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    let slack = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["surface"] == json!("slack"))
        .unwrap();
    assert_eq!(slack["source"], json!("instance"), "{rows}");

    let minted = server
        .minted(&operator.token, json!({ "provider": "glama" }))
        .await;
    let ws = workspace_id(&minted);
    let origin = server.store.workspace_origin(ws).await.unwrap();
    assert_eq!(origin, WorkspaceOrigin::Open);
    assert!(!maidan_store::instance_credentials::serves(origin, false));
    let (status, rows) = server
        .get(
            s(&minted, &["admin", "token"]),
            &format!("/workspaces/{}/connections", ws.0),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    let rows = rows.as_array().unwrap();
    let surfaces: Vec<&Value> = rows.iter().map(|row| &row["surface"]).collect();
    assert_eq!(
        surfaces,
        [&json!("github"), &json!("slack"), &json!("linear")]
    );
    for row in rows {
        assert_eq!(row["source"], Value::Null, "{row}");
    }
}
