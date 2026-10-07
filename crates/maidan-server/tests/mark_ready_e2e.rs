//! `POST /operator/github/mark-ready`: Soundcheck asks Maidan to flip a draft
//! agent pull request to ready for review.
//!
//! The fake GitHub below speaks just enough for the flip: `GET
//! /repos/{repo}/pulls/{n}` and `POST /graphql` with the
//! `markPullRequestReadyForReview` mutation. No live GitHub call is made
//! anywhere in this file.

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
    github::{GithubApiClient, GithubError, GithubGit},
    router,
    routes::MarkReadyGuardPass,
    AppState, FederationRuntime,
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
    node_id: String,
    /// `owner/name` of the head branch's repository; a fork differs from the
    /// base repository the fake always reports as `o/repo`.
    head_repo: String,
    open: bool,
}

#[derive(Default)]
struct FakeGithub {
    pulls: HashMap<i64, FakePull>,
    gets: Vec<i64>,
    /// Raw `POST /graphql` payloads, in order.
    graphql: Vec<Value>,
    /// When true the mutation answers `isDraft: true` without flipping, so
    /// the client's post-write verification can be pinned.
    ignore_flip: bool,
}

type Shared = Arc<Mutex<FakeGithub>>;

async fn github(State(fake): State<Shared>, method: Method, uri: Uri, body: Bytes) -> Response {
    let path = uri.path().to_string();
    let mut fake = fake.lock().unwrap();
    if path == "/graphql" && method == Method::POST {
        let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        fake.graphql.push(payload.clone());
        let node_id = payload
            .pointer("/variables/nodeId")
            .and_then(Value::as_str)
            .unwrap_or("");
        let pull = {
            let ignore_flip = fake.ignore_flip;
            let found = fake
                .pulls
                .values_mut()
                .find(|p| p.node_id == node_id)
                .map(|p| {
                    if !ignore_flip {
                        p.draft = false;
                    }
                    p.draft
                });
            found
        };
        match pull {
            None => {
                // GraphQL answers 200 with an `errors` array on rejection.
                return (
                    StatusCode::OK,
                    Json(json!({"errors": [{"message": "Could not resolve to a PullRequest with the ID"}]})),
                )
                    .into_response();
            }
            Some(still_draft) => {
                return (
                    StatusCode::OK,
                    Json(json!({"data": {"markPullRequestReadyForReview": {"pullRequest": {"isDraft": still_draft}}}})),
                )
                    .into_response();
            }
        }
    }
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
            "head": {"ref": p.head, "repo": {"full_name": p.head_repo}},
            "base": {"ref": p.base, "repo": {"full_name": "o/repo"}},
            "draft": p.draft,
            "state": if p.open { "open" } else { "closed" },
            "node_id": p.node_id,
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
        node_id: format!("PR_node_{number}"),
        head_repo: "o/repo".into(),
        open: true,
    }
}

// ---------------------------------------------------------------- client tests
//
// The client no longer guards: `set_pull_ready` is a bare GraphQL mutation
// and the single guard path (`flip_pull_ready_guarded`) lives with the
// caller. These tests pin the client's half of that contract — exactly one
// `POST /graphql` with the `markPullRequestReadyForReview` mutation keyed on
// the brief's node id, no read — plus the failure paths: a GraphQL `errors`
// answer and a mutation answer that leaves `isDraft: true` are both failed
// writes, never silent successes.
// The trait method takes a `MarkReadyGuardPass`; the tests mint one via the
// test-only constructor, the way production code never can.

async fn github_client() -> (GithubApiClient, Shared) {
    let fake: Shared = Arc::default();
    let base = spawn_github(fake.clone()).await;
    (
        GithubApiClient::with_base_url("test-token".into(), base).with_any_write_repo(),
        fake,
    )
}

#[tokio::test]
async fn a_write_to_a_repository_the_operator_did_not_name_is_refused_before_any_request() {
    let fake: Shared = Default::default();
    let base = spawn_github(fake.clone()).await;
    fake.lock()
        .unwrap()
        .pulls
        .insert(7, draft_pull(7, "feature/agent-x", "dev"));
    let node_id = fake.lock().unwrap().pulls[&7].node_id.clone();

    let elsewhere = GithubApiClient::with_base_url("test-token".into(), base.clone())
        .with_write_repos(["o/other".to_string()]);
    let err = elsewhere
        .set_pull_ready("o/repo", 7, &node_id, MarkReadyGuardPass::for_tests())
        .await
        .unwrap_err();
    assert!(matches!(err, GithubError::Refused(_)), "{err:?}");
    assert!(
        fake.lock().unwrap().graphql.is_empty(),
        "the refusal comes before any request"
    );

    let unconfigured = GithubApiClient::with_base_url("test-token".into(), base.clone());
    assert!(
        unconfigured
            .create_branch("o/repo", "feature/agent-x", "abc")
            .await
            .is_err(),
        "a client given no list writes nowhere"
    );

    // Names compare case-insensitively, as GitHub's do.
    let listed = GithubApiClient::with_base_url("test-token".into(), base)
        .with_write_repos(["O/Repo".to_string()]);
    listed
        .set_pull_ready("o/repo", 7, &node_id, MarkReadyGuardPass::for_tests())
        .await
        .unwrap();
    assert_eq!(fake.lock().unwrap().graphql.len(), 1);
}

#[tokio::test]
async fn set_pull_ready_posts_the_ready_mutation_and_reads_nothing() {
    let (client, fake) = github_client().await;
    fake.lock()
        .unwrap()
        .pulls
        .insert(7, draft_pull(7, "feature/agent-x", "dev"));
    let node_id = fake.lock().unwrap().pulls[&7].node_id.clone();

    client
        .set_pull_ready("o/repo", 7, &node_id, MarkReadyGuardPass::for_tests())
        .await
        .unwrap();
    let fake = fake.lock().unwrap();
    assert!(
        fake.gets.is_empty(),
        "no GET: the guard path reads before calling"
    );
    assert_eq!(fake.graphql.len(), 1, "one GraphQL call, no more");
    let payload = &fake.graphql[0];
    // The mutation is `markPullRequestReadyForReview`, keyed on the PR's
    // node id — no REST PATCH, no other mutation.
    let query = payload["query"].as_str().unwrap_or("");
    assert!(
        query.contains("markPullRequestReadyForReview"),
        "the ready mutation: {query}"
    );
    assert!(
        query.contains("pullRequestId"),
        "keyed on the node id: {query}"
    );
    assert_eq!(payload["variables"]["nodeId"], json!(node_id));
    assert!(!fake.pulls[&7].draft, "the fake flipped too");
}

#[tokio::test]
async fn set_pull_ready_fails_on_a_graphql_errors_answer() {
    let (client, fake) = github_client().await;
    fake.lock()
        .unwrap()
        .pulls
        .insert(7, draft_pull(7, "feature/agent-x", "dev"));

    // No pull carries this node id, so the fake answers GraphQL `errors`.
    let err = client
        .set_pull_ready(
            "o/repo",
            7,
            "PR_node_unknown",
            MarkReadyGuardPass::for_tests(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, GithubError::Http(_)),
        "a rejected mutation is a failed write: {err:?}"
    );
    assert!(err.to_string().contains("markPullRequestReadyForReview"));
}

#[tokio::test]
async fn set_pull_ready_fails_when_the_flip_is_unverified() {
    let (client, fake) = github_client().await;
    fake.lock()
        .unwrap()
        .pulls
        .insert(7, draft_pull(7, "feature/agent-x", "dev"));
    let node_id = fake.lock().unwrap().pulls[&7].node_id.clone();
    // The mutation answers 200 but leaves the PR a draft — the silent-no-op
    // class of failure the REST PATCH had. The client must not call it done.
    fake.lock().unwrap().ignore_flip = true;

    let err = client
        .set_pull_ready("o/repo", 7, &node_id, MarkReadyGuardPass::for_tests())
        .await
        .unwrap_err();
    assert!(
        matches!(err, GithubError::Http(_)),
        "an unverified flip is a failed write: {err:?}"
    );
    assert!(err.to_string().contains("isDraft"));
    assert!(
        fake.lock().unwrap().pulls[&7].draft,
        "the draft really is still a draft"
    );
}

#[tokio::test]
async fn pull_brief_reports_head_base_draft_and_node_id() {
    let (client, fake) = github_client().await;
    fake.lock()
        .unwrap()
        .pulls
        .insert(7, draft_pull(7, "feature/agent-x", "dev"));

    let brief = client.pull_brief("o/repo", 7).await.unwrap();
    assert_eq!(brief.number, 7);
    assert_eq!(brief.head, "feature/agent-x");
    assert_eq!(brief.base, "dev");
    assert!(brief.draft, "the guards refuse a non-draft");
    assert_eq!(brief.node_id, "PR_node_7", "the mutation keys on this");
    assert!(brief.open, "the fake reports an open pull request");
    assert!(brief.same_repo, "the head lives in the base repository");
}

#[tokio::test]
async fn pull_brief_errors_on_a_missing_pull() {
    let (client, _) = github_client().await;
    let err = client.pull_brief("o/repo", 999).await.unwrap_err();
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
    state.attach_github_sender(Arc::new(
        GithubApiClient::with_base_url("test-token".into(), github_base).with_any_write_repo(),
    ));
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
    // The operator designates the mark-ready app by id (MAIDAN_MARK_READY_APP_ID).
    state.mark_ready_app_id = Some(
        store
            .get_app_installation(soundcheck_install)
            .await
            .unwrap()
            .app_id,
    );
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
    assert!(
        body.to_string().contains("operator-designated app"),
        "{body}"
    );

    // The refusal is audited: this endpoint is the sole gate for a PR
    // mutation, so refused calls are recorded, not just marked ones.
    let events = h
        .store
        .list_audit_for_workspace(h.workspace_id, 10)
        .await
        .unwrap();
    let event = events
        .iter()
        .find(|e| e.action == "github.mark_ready" && e.metadata["outcome"] == json!("refused"))
        .expect("the app-gate refusal is audited");
    assert_eq!(event.metadata["pull_number"], json!(7));

    // Another app's token is not Soundcheck either.
    let (status, body) = post_mark_ready(&h, Some(&h.other_app_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Soundcheck passes the gate.
    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["marked_ready"], json!(true));
    assert!(
        h.fake.lock().unwrap().graphql.len() == 1,
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
        h.fake.lock().unwrap().graphql.is_empty(),
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
    // No allowlist entry on purpose: if the allowlist fired first, the
    // refusal would name the selector. The shape guard of the single guard
    // path must fire first and name the head.
    h.fake
        .lock()
        .unwrap()
        .pulls
        .insert(11, draft_pull(11, "feature/not-an-agent", "dev"));

    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 11).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let text = body.to_string();
    assert!(text.contains("does not match"), "{body}");
    assert!(
        !text.contains("o/repo@dev"),
        "the allowlist did not fire: {body}"
    );
    assert!(
        h.fake.lock().unwrap().graphql.is_empty(),
        "a refused flip writes nothing"
    );
}

#[tokio::test]
async fn a_look_alike_soundcheck_in_another_workspace_cannot_mark_ready() {
    let h = spawn().await;
    allow(&h, "o/repo@dev").await;
    h.fake
        .lock()
        .unwrap()
        .pulls
        .insert(14, draft_pull(14, "feature/agent-x", "dev"));

    // Workspace B builds its own app slugged `soundcheck` and blesses the same
    // repository and base in its own allowlist. Neither makes it the app the
    // operator designated.
    let ws_b = h
        .store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();
    let bot_b = h
        .store
        .create_member(NewMember {
            workspace_id: ws_b.id,
            handle: "bot-b".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let install_b = install_app(&h.store, ws_b.id, bot_b.id, "soundcheck").await;
    let secret_b = TokenSecret::generate();
    let bearer_b = mint(
        &h.store,
        ws_b.id,
        bot_b.id,
        Some(install_b),
        secret_b.as_str(),
    )
    .await;
    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: ws_b.id,
            surface: EgressSurface::GithubBranch,
            selector: "o/repo@dev".into(),
        })
        .await
        .unwrap();

    let (status, body) = post_mark_ready(&h, Some(&bearer_b), "o/repo", 14).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        h.fake.lock().unwrap().graphql.is_empty(),
        "a look-alike app writes nothing"
    );

    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 14).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the designated app still can: {body}"
    );
}

#[tokio::test]
async fn mark_ready_refuses_a_closed_pull() {
    let h = spawn().await;
    allow(&h, "o/repo@dev").await;
    let mut pull = draft_pull(13, "feature/agent-x", "dev");
    pull.open = false;
    h.fake.lock().unwrap().pulls.insert(13, pull);

    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 13).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains("closed"), "{body}");
    assert!(
        h.fake.lock().unwrap().graphql.is_empty(),
        "a closed pull request is never flipped"
    );
}

#[tokio::test]
async fn mark_ready_refuses_a_pull_from_a_fork() {
    let h = spawn().await;
    allow(&h, "o/repo@dev").await;
    // A fork can name its branch `feature/agent-*` and open a draft into an
    // allowlisted base; it is not one of the change flow's branches.
    let mut pull = draft_pull(12, "feature/agent-x", "dev");
    pull.head_repo = "stranger/repo".into();
    h.fake.lock().unwrap().pulls.insert(12, pull);

    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 12).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains("fork"), "{body}");
    assert!(
        h.fake.lock().unwrap().graphql.is_empty(),
        "a fork's pull request is never flipped"
    );
}

#[tokio::test]
async fn mark_ready_checks_the_allowlist_against_the_fresh_base() {
    let h = spawn().await;
    allow(&h, "o/repo@dev").await;
    // The PR targets staging while the allowlist blesses dev. The refusal
    // must name the PR's actual base, proving the allowlist ran on the fresh
    // read rather than on a caller-supplied base.
    h.fake
        .lock()
        .unwrap()
        .pulls
        .insert(12, draft_pull(12, "feature/agent-x", "staging"));

    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 12).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains("o/repo@staging"), "{body}");
    assert!(
        h.fake.lock().unwrap().graphql.is_empty(),
        "a refused flip writes nothing"
    );
}

#[tokio::test]
async fn mark_ready_enforces_the_per_repo_base_map() {
    let h = spawn().await;
    // wax flips only into dev, even with the allowlist blessing main.
    allow(&h, "david-engelmann/wax@main").await;
    h.fake
        .lock()
        .unwrap()
        .pulls
        .insert(13, draft_pull(13, "feature/agent-x", "main"));

    let (status, body) =
        post_mark_ready(&h, Some(&h.soundcheck_bearer), "david-engelmann/wax", 13).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains("only into `dev`"), "{body}");
    assert!(
        h.fake.lock().unwrap().graphql.is_empty(),
        "a refused flip writes nothing"
    );

    // agent-skills flips only into main.
    allow(&h, "david-engelmann/agent-skills@dev").await;
    h.fake
        .lock()
        .unwrap()
        .pulls
        .insert(14, draft_pull(14, "feature/agent-x", "dev"));

    let (status, body) = post_mark_ready(
        &h,
        Some(&h.soundcheck_bearer),
        "david-engelmann/agent-skills",
        14,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.to_string().contains("only into `main`"), "{body}");
    assert!(
        h.fake.lock().unwrap().graphql.is_empty(),
        "a refused flip writes nothing"
    );
}

#[tokio::test]
async fn mark_ready_rejects_a_bad_request_and_audits_it() {
    let h = spawn().await;

    let (status, _) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "", 7).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "empty repo");
    let (status, _) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 0).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "non-positive pull number");

    let events = h
        .store
        .list_audit_for_workspace(h.workspace_id, 10)
        .await
        .unwrap();
    let refused: Vec<_> = events
        .iter()
        .filter(|e| e.action == "github.mark_ready" && e.metadata["outcome"] == json!("refused"))
        .collect();
    assert_eq!(refused.len(), 2, "both bad requests are audited as refused");
}

#[tokio::test]
async fn mark_ready_rejects_a_misshapen_repo_and_audits_it() {
    let h = spawn().await;

    for bad in [
        "o",
        "o/repo/extra",
        "o/re po",
        "o/r*po",
        "/repo",
        "o/",
        "o//repo",
    ] {
        let (status, _) = post_mark_ready(&h, Some(&h.soundcheck_bearer), bad, 7).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "repo {bad:?}");
    }

    // A well-shaped repo passes validation and reaches the guards (here the
    // allowlist, which refuses it).
    let (status, _) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a well-shaped repo reaches the guards"
    );

    let events = h
        .store
        .list_audit_for_workspace(h.workspace_id, 10)
        .await
        .unwrap();
    let shape_refused: Vec<_> = events
        .iter()
        .filter(|e| {
            e.action == "github.mark_ready"
                && e.metadata["outcome"] == json!("refused")
                && e.metadata["reason"] == json!("repo must be owner/name")
        })
        .collect();
    assert_eq!(
        shape_refused.len(),
        7,
        "every misshapen repo is audited as refused"
    );
}

#[tokio::test]
async fn mark_ready_404s_a_missing_pull_and_audits_the_failure() {
    let h = spawn().await;
    allow(&h, "o/repo@dev").await;

    let (status, _) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 999).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let events = h
        .store
        .list_audit_for_workspace(h.workspace_id, 10)
        .await
        .unwrap();
    let event = events
        .iter()
        .find(|e| e.action == "github.mark_ready" && e.metadata["outcome"] == json!("failed"))
        .expect("the failure is audited");
    assert_eq!(event.metadata["pull_number"], json!(999));
}

#[tokio::test]
async fn mark_ready_audits_refusals_marks_and_retries() {
    let h = spawn().await;

    // Refused: no allowlist entry.
    let (status, _) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Marked, then already-ready on the retry.
    allow(&h, "o/repo@dev").await;
    let (status, _) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["marked_ready"], json!(false));

    let events = h
        .store
        .list_audit_for_workspace(h.workspace_id, 10)
        .await
        .unwrap();
    let outcomes: Vec<String> = events
        .iter()
        .filter(|e| e.action == "github.mark_ready")
        .filter_map(|e| e.metadata["outcome"].as_str().map(str::to_string))
        .collect();
    for want in ["refused", "marked", "already_ready"] {
        assert!(outcomes.contains(&want.to_string()), "{outcomes:?}");
    }
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
    // The fake flipped the draft on the first mutation; the retry finds it ready.
    let (status, body) = post_mark_ready(&h, Some(&h.soundcheck_bearer), "o/repo", 7).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["marked_ready"], json!(false));
    assert_eq!(body["reason"], json!("already ready"));
    assert_eq!(
        h.fake.lock().unwrap().graphql.len(),
        1,
        "the retry writes nothing"
    );
}

/// `MarkReadyGuardPass::for_tests` is public because integration tests need
/// it, so this is what keeps production code from minting a pass that skips
/// the guards: no file under any crate's `src/` may call it.
#[test]
fn no_source_file_mints_a_guard_pass_for_tests() {
    fn visit(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                if text.contains("MarkReadyGuardPass::for_tests") {
                    hits.push(path.display().to_string());
                }
            }
        }
    }
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap() {
        let src = entry.unwrap().path().join("src");
        if src.is_dir() {
            visit(&src, &mut hits);
        }
    }
    assert!(
        hits.is_empty(),
        "production code must not mint a guard pass: {hits:?}"
    );
}
