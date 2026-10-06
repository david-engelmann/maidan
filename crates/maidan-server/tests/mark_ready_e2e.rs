//! `POST /operator/github/mark-ready`: Soundcheck asks Maidan to flip a draft
//! agent pull request to ready for review.
//!
//! The fake GitHub below speaks just enough REST for the flip: `GET
//! /repos/{repo}/pulls/{n}` and `PATCH` with `{"draft": false}`. No live
//! GitHub call is made anywhere in this file.

#![allow(clippy::disallowed_methods)]

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc, Mutex},
};

use axum::{
    body::Bytes,
    extract::State,
    http::{Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Json, Router,
};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{
    github::{GithubApiClient, GithubGit},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EgressSurface, MemberKind, NewApiToken, NewApp, NewAppInstallation, NewEgressTarget, NewMember,
    NewWorkspace,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

// ---------------------------------------------------------------- fake GitHub

#[derive(Clone, Debug)]
struct FakePull {
    number: i64,
    head: String,
    base: String,
    draft: bool,
}

#[derive(Default)]
struct FakeGithub {
    pulls: HashMap<i64, FakePull>,
    gets: Vec<i64>,
    patches: Vec<(i64, Value)>,
}

type Shared = Arc<Mutex<FakeGithub>>;

async fn github(State(fake): State<Shared>, method: Method, uri: Uri, body: Bytes) -> Response {
    let path = uri.path().to_string();
    let mut fake = fake.lock().unwrap();
    // `/repos/{owner}/{name}/pulls/{n}` — the owner and name are not
    // interpreted; the number selects the pull.
    let number: i64 = path
        .rsplit('/')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1);
    if !path.contains("/pulls/") {
        return (StatusCode::NOT_FOUND, Json(json!({}))).into_response();
    }
    let pull_json = |p: &FakePull| {
        json!({
            "number": p.number,
            "head": {"ref": p.head},
            "base": {"ref": p.base},
            "draft": p.draft,
        })
    };
    match method.as_str() {
        "GET" => {
            fake.gets.push(number);
            match fake.pulls.get(&number) {
                Some(p) => (StatusCode::OK, Json(pull_json(p))).into_response(),
                None => {
                    (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))).into_response()
                }
            }
        }
        "PATCH" => {
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            fake.patches.push((number, body.clone()));
            match fake.pulls.get_mut(&number) {
                Some(p) => {
                    if body.get("draft") == Some(&json!(false)) {
                        p.draft = false;
                    }
                    (StatusCode::OK, Json(pull_json(p))).into_response()
                }
                None => {
                    (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))).into_response()
                }
            }
        }
        _ => (StatusCode::NOT_FOUND, Json(json!({}))).into_response(),
    }
}

async fn spawn_github(fake: Shared) -> String {
    let app = Router::new().fallback(any(github)).with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn draft_pull(number: i64, head: &str, base: &str) -> FakePull {
    FakePull {
        number,
        head: head.into(),
        base: base.into(),
        draft: true,
    }
}

// ---------------------------------------------------------------- client tests

async fn github_client() -> (GithubApiClient, Shared) {
    let fake: Shared = Arc::default();
    let base = spawn_github(fake.clone()).await;
    (
        GithubApiClient::with_base_url("test-token".into(), base),
        fake,
    )
}

#[tokio::test]
async fn mark_ready_flips_a_draft_and_touches_nothing_else() {
    let (client, fake) = github_client().await;
    fake.lock()
        .unwrap()
        .pulls
        .insert(7, draft_pull(7, "feature/agent-x", "dev"));

    let outcome = client.mark_pull_request_ready("o/repo", 7).await.unwrap();
    assert_eq!(
        outcome,
        maidan_server::github::MarkReadyOutcome::Marked {
            number: 7,
            head: "feature/agent-x".into(),
            base: "dev".into(),
        },
        "{outcome:?}"
    );
    let fake = fake.lock().unwrap();
    assert_eq!(fake.patches.len(), 1, "one PATCH, no more");
    assert_eq!(fake.patches[0].0, 7);
    // The flip is exactly `{"draft": false}`: no title, body, base or head
    // is rewritten by the call.
    assert_eq!(fake.patches[0].1, json!({"draft": false}));
    assert!(!fake.pulls[&7].draft, "the fake flipped too");
}

#[tokio::test]
async fn mark_ready_refuses_a_head_that_is_not_an_agent_branch() {
    let (client, fake) = github_client().await;
    fake.lock()
        .unwrap()
        .pulls
        .insert(8, draft_pull(8, "hotfix/urgent", "dev"));

    let outcome = client.mark_pull_request_ready("o/repo", 8).await.unwrap();
    assert!(
        matches!(outcome, maidan_server::github::MarkReadyOutcome::Refused(_)),
        "{outcome:?}"
    );
    assert!(
        fake.lock().unwrap().patches.is_empty(),
        "a refused flip writes nothing"
    );
}

#[tokio::test]
async fn mark_ready_refuses_a_forbidden_base() {
    let (client, fake) = github_client().await;
    fake.lock()
        .unwrap()
        .pulls
        .insert(9, draft_pull(9, "feature/agent-x", "prod"));

    let outcome = client.mark_pull_request_ready("o/repo", 9).await.unwrap();
    assert!(
        matches!(outcome, maidan_server::github::MarkReadyOutcome::Refused(_)),
        "{outcome:?}"
    );
    assert!(
        fake.lock().unwrap().patches.is_empty(),
        "a refused flip writes nothing"
    );
}

#[tokio::test]
async fn mark_ready_is_a_noop_on_an_already_ready_pull() {
    let (client, fake) = github_client().await;
    fake.lock().unwrap().pulls.insert(
        10,
        FakePull {
            number: 10,
            head: "feature/agent-x".into(),
            base: "dev".into(),
            draft: false,
        },
    );

    let outcome = client.mark_pull_request_ready("o/repo", 10).await.unwrap();
    assert_eq!(
        outcome,
        maidan_server::github::MarkReadyOutcome::AlreadyReady { number: 10 }
    );
    assert!(
        fake.lock().unwrap().patches.is_empty(),
        "an already-ready pull is not rewritten"
    );
}

#[tokio::test]
async fn mark_ready_errors_on_a_missing_pull() {
    let (client, _) = github_client().await;
    let err = client
        .mark_pull_request_ready("o/repo", 999)
        .await
        .unwrap_err();
    assert!(err.is_not_found(), "{err:?}");
}

// ---------------------------------------------------------------- route tests

struct Harness {
    addr: SocketAddr,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    fake: Shared,
    workspace_id: maidan_types::WorkspaceId,
    soundcheck_bearer: String,
    other_app_bearer: String,
    member_bearer: String,
}

fn base(h: &Harness) -> String {
    format!("http://{}", h.addr)
}

async fn mint(
    store: &Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    member: maidan_types::MemberId,
    installation: Option<maidan_types::AppInstallationId>,
    secret: &str,
) -> String {
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: installation,
            token_hash: hash_secret(secret),
            label: Some("test".into()),
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.to_string()
}

async fn install_app(
    store: &Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    bot: maidan_types::MemberId,
    slug: &str,
) -> maidan_types::AppInstallationId {
    let app = store
        .create_app(NewApp {
            workspace_id: ws,
            slug: slug.into(),
            name: slug.into(),
            description: None,
            created_by: bot,
        })
        .await
        .unwrap();
    store
        .create_app_installation(NewAppInstallation {
            app_id: app.id,
            workspace_id: ws,
            bot_member_id: bot,
            granted_capabilities: vec![capability::WORKSPACE_READ.into()],
        })
        .await
        .unwrap()
        .id
}

async fn spawn() -> Harness {
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

    let fake: Shared = Arc::default();
    let github_base = spawn_github(fake.clone()).await;
    let mut state = AppState::new(
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
    state.attach_github_sender(Arc::new(GithubApiClient::with_base_url(
        "test-token".into(),
        github_base,
    )));
    std::mem::forget(dir);

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "human".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();

    let soundcheck_install = install_app(&store, ws.id, bot.id, "soundcheck").await;
    let other_install = install_app(&store, ws.id, bot.id, "other-app").await;
    let soundcheck_secret = TokenSecret::generate();
    let other_secret = TokenSecret::generate();
    let member_secret = TokenSecret::generate();
    let soundcheck_bearer = mint(
        &store,
        ws.id,
        bot.id,
        Some(soundcheck_install),
        soundcheck_secret.as_str(),
    )
    .await;
    let other_app_bearer = mint(
        &store,
        ws.id,
        bot.id,
        Some(other_install),
        other_secret.as_str(),
    )
    .await;
    let member_bearer = mint(&store, ws.id, member.id, None, member_secret.as_str()).await;

    fake.lock()
        .unwrap()
        .pulls
        .insert(7, draft_pull(7, "feature/agent-x", "dev"));

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        addr,
        client: reqwest::Client::new(),
        store,
        fake,
        workspace_id: ws.id,
        soundcheck_bearer,
        other_app_bearer,
        member_bearer,
    }
}

async fn post_mark_ready(
    h: &Harness,
    bearer: Option<&str>,
    repo: &str,
    n: i64,
) -> (StatusCode, Value) {
    let mut req = h
        .client
        .post(format!("{}/operator/github/mark-ready", base(h)))
        .json(&json!({"repo": repo, "pull_number": n}));
    if let Some(b) = bearer {
        req = req.bearer_auth(b);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

async fn allow(h: &Harness, selector: &str) {
    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: h.workspace_id,
            surface: EgressSurface::GithubBranch,
            selector: selector.into(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn only_the_soundcheck_app_may_mark_ready() {
    let h = spawn().await;
    allow(&h, "o/repo@dev").await;

    // No token at all.
    let (status, _) = post_mark_ready(&h, None, "o/repo", 7).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no token");

    // A member token is not an app token.
    let (status, body) = post_mark_ready(&h, Some(&h.member_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains("soundcheck"), "{body}");

    // Another app's token is not Soundcheck either.
    let (status, body) = post_mark_ready(&h, Some(&h.other_app_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Soundcheck passes the gate.
    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["marked_ready"], json!(true));
    assert!(
        h.fake.lock().unwrap().patches.len() == 1,
        "the flip landed exactly once"
    );
}

#[tokio::test]
async fn mark_ready_needs_the_allowlist() {
    let h = spawn().await;

    // No allowlist entry: refused, naming the selector.
    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains("o/repo@dev"), "{body}");
    assert!(
        h.fake.lock().unwrap().patches.is_empty(),
        "nothing is written before the allowlist"
    );

    allow(&h, "o/repo@dev").await;
    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["marked_ready"], json!(true));
}

#[tokio::test]
async fn mark_ready_refuses_a_non_agent_head() {
    let h = spawn().await;
    allow(&h, "o/repo@dev").await;
    h.fake
        .lock()
        .unwrap()
        .pulls
        .insert(11, draft_pull(11, "main", "dev"));

    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 11).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        h.fake.lock().unwrap().patches.is_empty(),
        "a refused flip writes nothing"
    );
}

#[tokio::test]
async fn mark_ready_audits_every_call() {
    let h = spawn().await;
    allow(&h, "o/repo@dev").await;

    let (status, _) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::OK);

    let events = h
        .store
        .list_audit_for_workspace(h.workspace_id, 10)
        .await
        .unwrap();
    let event = events
        .iter()
        .find(|e| e.action == "github.mark_ready")
        .expect("the flip is audited");
    assert_eq!(event.metadata["outcome"], json!("marked"));
    assert_eq!(event.metadata["repo"], json!("o/repo"));
    assert_eq!(event.metadata["pull_number"], json!(7));
    assert_eq!(event.metadata["head"], json!("feature/agent-x"));
    assert_eq!(event.metadata["base"], json!("dev"));
}

#[tokio::test]
async fn mark_ready_is_idempotent_for_soundcheck_retries() {
    let h = spawn().await;
    allow(&h, "o/repo@dev").await;

    let (first, _) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(first, StatusCode::OK);
    // The fake flipped the draft on the first PATCH; the retry finds it ready.
    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["marked_ready"], json!(false));
    assert_eq!(body["reason"], json!("already ready"));
    assert_eq!(
        h.fake.lock().unwrap().patches.len(),
        1,
        "the retry writes nothing"
    );
}
