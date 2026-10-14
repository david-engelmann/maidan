//! `/operator/github/enqueue`: the mark-ready app labels a ready agent pull
//! request for its repository's merge queue (`POST`), reads whether it is
//! labelled (`GET`), and takes the label off (`DELETE`). Enqueue runs every
//! guard on one fresh read: a label is configured for the repository, the PR
//! is a ready `feature/agent-*` head into its pinned base and not a fork, the
//! allowlist names it, and its head is the commit the change flow delivered.
//! Dequeue skips the head comparison, since the queue rebases. Against a fake
//! GitHub.

// The fake GitHub is a plain axum server, not the API (see clippy.toml).
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
use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{github::GithubApiClient, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EgressSurface, EgressTarget, MarkReadyBases, MemberKind, MergeQueueLabels, NewApiToken, NewApp,
    NewAppInstallation, NewChannel, NewEgressTarget, NewMember, NewThread, NewWorkspace,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const REPO: &str = "example-org/example-repo";
const DELIVERED: &str = "1111111111111111111111111111111111111111";
const LABEL: &str = "merge-queue";

#[derive(Clone)]
struct FakePull {
    head: String,
    head_sha: String,
    draft: bool,
    labels: Vec<String>,
}

#[derive(Default)]
struct FakeGithub {
    pulls: HashMap<i64, FakePull>,
    label_posts: Vec<(i64, Value)>,
    label_deletes: Vec<(i64, String)>,
}

type Shared = Arc<Mutex<FakeGithub>>;

async fn github(State(fake): State<Shared>, method: Method, uri: Uri, body: Bytes) -> Response {
    let path = uri.path().to_string();
    let mut fake = fake.lock().unwrap();
    let parts: Vec<&str> = path.split('/').collect();
    // /repos/{owner}/{name}/issues/{n}/labels
    if method == Method::POST && path.ends_with("/labels") {
        let number: i64 = parts[parts.len() - 2].parse().unwrap_or(-1);
        let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        fake.label_posts.push((number, payload.clone()));
        let Some(pull) = fake.pulls.get_mut(&number) else {
            return (StatusCode::NOT_FOUND, Json(json!({}))).into_response();
        };
        for label in payload["labels"].as_array().into_iter().flatten() {
            let label = label.as_str().unwrap_or_default().to_string();
            if !pull.labels.contains(&label) {
                pull.labels.push(label);
            }
        }
        let answer: Vec<Value> = pull.labels.iter().map(|l| json!({"name": l})).collect();
        return (StatusCode::OK, Json(Value::Array(answer))).into_response();
    }
    // /repos/{owner}/{name}/issues/{n}/labels/{label}
    if method == Method::DELETE && path.contains("/labels/") {
        let number: i64 = parts[parts.len() - 3].parse().unwrap_or(-1);
        let label = urlencoding::decode(parts[parts.len() - 1])
            .unwrap()
            .into_owned();
        fake.label_deletes.push((number, label.clone()));
        let Some(pull) = fake.pulls.get_mut(&number) else {
            return (StatusCode::NOT_FOUND, Json(json!({}))).into_response();
        };
        if !pull.labels.contains(&label) {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"message": "Label does not exist"})),
            )
                .into_response();
        }
        pull.labels.retain(|l| l != &label);
        let answer: Vec<Value> = pull.labels.iter().map(|l| json!({"name": l})).collect();
        return (StatusCode::OK, Json(Value::Array(answer))).into_response();
    }
    // /repos/{owner}/{name}/pulls/{n}
    if method == Method::GET && path.contains("/pulls/") {
        let number: i64 = parts.last().and_then(|n| n.parse().ok()).unwrap_or(-1);
        return match fake.pulls.get(&number) {
            Some(p) => (
                StatusCode::OK,
                Json(json!({
                    "number": number,
                    "head": {"ref": p.head, "sha": p.head_sha, "repo": {"full_name": REPO}},
                    "base": {"ref": "dev", "repo": {"full_name": REPO}},
                    "draft": p.draft,
                    "state": "open",
                    "node_id": format!("PR_{number}"),
                    "labels": p.labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(),
                })),
            )
                .into_response(),
            None => (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))).into_response(),
        };
    }
    (StatusCode::NOT_FOUND, Json(json!({}))).into_response()
}

struct Harness {
    base: String,
    fake: Shared,
    app_bearer: String,
    member_bearer: String,
}

async fn token(
    store: &Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    member: maidan_types::MemberId,
    installation: Option<maidan_types::AppInstallationId>,
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: installation,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

/// A workspace whose change flow delivered `DELIVERED` to pull request 7, and
/// a merge-queue label for `REPO` only.
async fn spawn(pull: FakePull) -> Harness {
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
    let fake: Shared = Arc::default();
    fake.lock().unwrap().pulls.insert(7, pull);
    let github_app = Router::new().fallback(any(github)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let github_base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, github_app).await.unwrap() });

    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
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
    let app = store
        .create_app(NewApp {
            workspace_id: ws.id,
            slug: "mark-ready-app".into(),
            name: "mark-ready-app".into(),
            description: None,
            created_by: bot.id,
        })
        .await
        .unwrap();
    let install = store
        .create_app_installation(NewAppInstallation {
            app_id: app.id,
            workspace_id: ws.id,
            bot_member_id: bot.id,
            granted_capabilities: vec![capability::WORKSPACE_READ.into()],
        })
        .await
        .unwrap();
    state.mark_ready_app_id = Some(app.id);
    state.mark_ready_bases = MarkReadyBases::parse(&format!("{REPO}=dev")).unwrap();
    state.merge_queue_labels = MergeQueueLabels::parse(&format!("{REPO}={LABEL}")).unwrap();
    store
        .allow_egress_target(NewEgressTarget {
            workspace_id: ws.id,
            surface: EgressSurface::GithubBranch,
            selector: format!("{REPO}@dev"),
        })
        .await
        .unwrap();

    // The change flow's delivery of DELIVERED to pull request 7.
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
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
            title: None,
            description: None,
        })
        .await
        .unwrap();
    let target = EgressTarget::GithubBranch {
        repo: REPO.into(),
        branch: "feature/agent-x".into(),
    };
    let row = store
        .arm_result_delivery(thread.id, &target, Utc::now())
        .await
        .unwrap()
        .unwrap();
    store
        .mark_result_delivered(row.id, Some(&format!("{DELIVERED}#7")), Utc::now())
        .await
        .unwrap();

    let app_bearer = token(&store, ws.id, bot.id, Some(install.id)).await;
    let member_bearer = token(&store, ws.id, bot.id, None).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        fake,
        app_bearer,
        member_bearer,
    }
}

fn ready(head_sha: &str) -> FakePull {
    FakePull {
        head: "feature/agent-x".into(),
        head_sha: head_sha.into(),
        draft: false,
        labels: vec![],
    }
}

async fn enqueue(h: &Harness, bearer: &str, repo: &str) -> (StatusCode, Value) {
    let resp = reqwest::Client::new()
        .post(format!("{}/operator/github/enqueue", h.base))
        .bearer_auth(bearer)
        .json(&json!({"repo": repo, "pull_number": 7}))
        .send()
        .await
        .unwrap();
    (resp.status(), resp.json().await.unwrap_or(Value::Null))
}

async fn status(h: &Harness) -> Value {
    reqwest::Client::new()
        .get(format!(
            "{}/operator/github/enqueue?repo={REPO}&pull_number=7",
            h.base
        ))
        .bearer_auth(&h.app_bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn dequeue(h: &Harness) -> (StatusCode, Value) {
    let resp = reqwest::Client::new()
        .delete(format!(
            "{}/operator/github/enqueue?repo={REPO}&pull_number=7",
            h.base
        ))
        .bearer_auth(&h.app_bearer)
        .send()
        .await
        .unwrap();
    (resp.status(), resp.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_ready_delivered_pull_request_is_labelled_once() {
    let h = spawn(ready(DELIVERED)).await;
    assert_eq!(status(&h).await["applied"], json!(false));

    let (code, body) = enqueue(&h, &h.app_bearer, REPO).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(body["label"], LABEL);
    assert_eq!(body["applied"], json!(true));
    assert_eq!(status(&h).await["applied"], json!(true));

    // Again: already labelled, no second write.
    let (code, body) = enqueue(&h, &h.app_bearer, REPO).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(body["reason"], "already labelled");
    let posts = h.fake.lock().unwrap().label_posts.clone();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].1, json!({"labels": [LABEL]}));
}

#[tokio::test]
async fn a_head_moved_past_the_delivered_commit_is_refused_naming_both() {
    let pushed = "2222222222222222222222222222222222222222";
    let h = spawn(ready(pushed)).await;
    let (code, body) = enqueue(&h, &h.app_bearer, REPO).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    let detail = body["detail"].as_str().unwrap();
    assert!(
        detail.contains(pushed) && detail.contains(DELIVERED),
        "{detail}"
    );
    assert!(h.fake.lock().unwrap().label_posts.is_empty());
}

#[tokio::test]
async fn a_draft_an_unlisted_repository_and_another_caller_are_refused() {
    let h = spawn(FakePull {
        draft: true,
        ..ready(DELIVERED)
    })
    .await;
    let (code, body) = enqueue(&h, &h.app_bearer, REPO).await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(body["detail"].as_str().unwrap().contains("draft"), "{body}");

    let (code, body) = enqueue(&h, &h.app_bearer, "example-org/human-merged").await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("MAIDAN_MERGE_QUEUE_LABELS"),
        "{body}"
    );

    let (code, _) = enqueue(&h, &h.member_bearer, REPO).await;
    assert_eq!(
        code,
        StatusCode::FORBIDDEN,
        "only the mark-ready app enqueues"
    );
    assert!(h.fake.lock().unwrap().label_posts.is_empty());
}

#[tokio::test]
async fn dequeue_takes_the_label_off_whatever_the_head_now_is() {
    // The queue rebased the head after enqueue: dequeue does not compare it.
    let rebased = "3333333333333333333333333333333333333333";
    let h = spawn(FakePull {
        labels: vec![LABEL.into()],
        ..ready(rebased)
    })
    .await;
    let (code, body) = dequeue(&h).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(body["applied"], json!(false));
    assert_eq!(status(&h).await["applied"], json!(false));
    assert_eq!(h.fake.lock().unwrap().label_deletes.len(), 1);

    // Again: not labelled, no second write.
    let (code, body) = dequeue(&h).await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(body["reason"], "was not labelled");
    assert_eq!(h.fake.lock().unwrap().label_deletes.len(), 1);
}
