//! The Slack change flow end to end: a `pi.change.result/1` routed through the
//! real result-delivery path and egress worker, with the production
//! `GithubApiClient` pointed at an in-memory fake of GitHub's Git Data and
//! Pulls APIs, and a recording Slack sender.
//!
//! The fake keeps real state (refs, commits, trees, blobs, pulls), so the
//! tests assert what is on the "GitHub" afterwards, not only which calls were
//! made.

// The fake GitHub is a plain axum server, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc, Mutex},
};

use axum::{
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Json, Router,
};
use base64::Engine as _;
use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    egress_worker, github::GithubApiClient, notification_router, slack::SlackError,
    slack::SlackSender, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    status, ChannelId, EgressSurface, EgressTarget, Event, ExternalRef, MemberId, MemberKind,
    NewChannel, NewEgressTarget, NewMember, NewMessage, NewThread, NewWorkspace, ResultDelivery,
    ThreadId, WorkspaceId, PI_CHANGE_RESULT_KIND, WAITER_RESULT_SCHEMA,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const REPO: &str = "beatgig/bgv3";
const BRANCH: &str = "feature/agent-fix-greeting-1a2b";
const CHANNEL: &str = "C0SOUNDCHK";
const THREAD_TS: &str = "1759500000.000100";
const TOKEN: &str = "ghp_test_token_value";

// ---------------------------------------------------------------- fake GitHub

#[derive(Clone, Debug)]
struct FakeCommit {
    tree: String,
    parents: Vec<String>,
    message: String,
}

#[derive(Clone, Debug)]
struct FakePull {
    number: i64,
    head: String,
    base: String,
    title: String,
    body: String,
    draft: bool,
}

type Tree = BTreeMap<String, (String, String)>;

#[derive(Default)]
struct Fake {
    next: u64,
    refs: HashMap<String, String>,
    commits: HashMap<String, FakeCommit>,
    trees: HashMap<String, Tree>,
    blobs: HashMap<String, Vec<u8>>,
    pulls: Vec<FakePull>,
    requests: Vec<String>,
    /// Fail `POST /pulls` with a body that echoes the token, as a careless
    /// proxy might.
    fail_pulls: bool,
}

impl Fake {
    fn sha(&mut self) -> String {
        self.next += 1;
        format!("{:040x}", 0xabc000 + self.next)
    }

    fn blob(&mut self, content: &[u8]) -> String {
        let sha = self.sha();
        self.blobs.insert(sha.clone(), content.to_vec());
        sha
    }

    fn commit_files(&mut self, files: &[(&str, &str)], parents: Vec<String>) -> String {
        let mut tree = Tree::new();
        for (path, content) in files {
            let blob = self.blob(content.as_bytes());
            tree.insert(path.to_string(), ("100644".into(), blob));
        }
        let tree_sha = self.sha();
        self.trees.insert(tree_sha.clone(), tree);
        let sha = self.sha();
        self.commits.insert(
            sha.clone(),
            FakeCommit {
                tree: tree_sha,
                parents,
                message: "seed".into(),
            },
        );
        sha
    }

    fn file(&self, commit: &str, path: &str) -> Option<String> {
        let tree = &self.trees[&self.commits[commit].tree];
        let (_, blob) = tree.get(path)?;
        Some(String::from_utf8(self.blobs[blob].clone()).unwrap())
    }

    fn writes(&self) -> Vec<&String> {
        self.requests
            .iter()
            .filter(|r| !r.starts_with("GET "))
            .collect()
    }
}

type Shared = Arc<Mutex<Fake>>;

fn reply(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

async fn github(
    State(fake): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let mut fake = fake.lock().unwrap();
    let path = urlencoding::decode(uri.path()).unwrap().into_owned();
    fake.requests.push(format!("{method} {path}"));
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let Some(rest) = path.strip_prefix(&format!("/repos/{REPO}/")) else {
        return reply(StatusCode::NOT_FOUND, json!({}));
    };
    let not_found = || reply(StatusCode::NOT_FOUND, json!({"message": "Not Found"}));
    match (method.as_str(), rest) {
        ("GET", r) if r.starts_with("git/ref/heads/") => {
            match fake.refs.get(&r["git/ref/heads/".len()..]) {
                Some(sha) => reply(StatusCode::OK, json!({"object": {"sha": sha}})),
                None => not_found(),
            }
        }
        ("POST", "git/refs") => {
            let name = body["ref"]
                .as_str()
                .unwrap()
                .trim_start_matches("refs/heads/");
            let sha = body["sha"].as_str().unwrap();
            if fake.refs.contains_key(name) || !fake.commits.contains_key(sha) {
                return reply(StatusCode::UNPROCESSABLE_ENTITY, json!({}));
            }
            fake.refs.insert(name.into(), sha.into());
            reply(StatusCode::CREATED, json!({"ref": body["ref"]}))
        }
        ("PATCH", r) if r.starts_with("git/refs/heads/") => {
            let name = &r["git/refs/heads/".len()..];
            let sha = body["sha"].as_str().unwrap();
            let Some(current) = fake.refs.get(name).cloned() else {
                return not_found();
            };
            let fast_forward = fake
                .commits
                .get(sha)
                .is_some_and(|c| c.parents.contains(&current));
            assert_eq!(body["force"], json!(false), "the flow never force-pushes");
            if !fast_forward {
                return reply(StatusCode::UNPROCESSABLE_ENTITY, json!({}));
            }
            fake.refs.insert(name.into(), sha.into());
            reply(StatusCode::OK, json!({}))
        }
        ("GET", r) if r.starts_with("git/commits/") => {
            let sha = &r["git/commits/".len()..];
            match fake.commits.get(sha) {
                Some(c) => reply(
                    StatusCode::OK,
                    json!({
                        "sha": sha,
                        "tree": {"sha": c.tree},
                        "parents": c.parents.iter().map(|p| json!({"sha": p})).collect::<Vec<_>>(),
                        "message": c.message,
                    }),
                ),
                None => not_found(),
            }
        }
        ("GET", r) if r.starts_with("contents/") => {
            let file = &r["contents/".len()..];
            let at = uri
                .query()
                .and_then(|q| q.strip_prefix("ref="))
                .unwrap_or_default();
            // Raw bytes only for a request that asks for nothing else; any
            // other `Accept` gets the JSON envelope, as GitHub may send it.
            let raw_only = headers
                .get_all("accept")
                .iter()
                .map(|v| v.to_str().unwrap_or_default())
                .eq(["application/vnd.github.raw+json"]);
            match fake.commits.get(at).map(|c| c.tree.clone()) {
                Some(tree) => match fake.trees[&tree].get(file) {
                    Some((_, blob)) if raw_only => fake.blobs[blob].clone().into_response(),
                    Some((_, blob)) => reply(
                        StatusCode::OK,
                        json!({"type": "file", "encoding": "base64", "sha": blob}),
                    ),
                    None => not_found(),
                },
                None => not_found(),
            }
        }
        ("POST", "git/blobs") => {
            assert_eq!(body["encoding"], "base64");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(body["content"].as_str().unwrap())
                .unwrap();
            let sha = fake.blob(&bytes);
            reply(StatusCode::CREATED, json!({"sha": sha}))
        }
        ("POST", "git/trees") => {
            let base = body["base_tree"].as_str().unwrap();
            let mut tree = fake.trees[base].clone();
            for entry in body["tree"].as_array().unwrap() {
                let path = entry["path"].as_str().unwrap().to_string();
                match entry["sha"].as_str() {
                    Some(blob) => {
                        tree.insert(path, (entry["mode"].as_str().unwrap().into(), blob.into()));
                    }
                    None => {
                        tree.remove(&path);
                    }
                }
            }
            let sha = fake.sha();
            fake.trees.insert(sha.clone(), tree);
            reply(StatusCode::CREATED, json!({"sha": sha}))
        }
        ("POST", "git/commits") => {
            let sha = fake.sha();
            let commit = FakeCommit {
                tree: body["tree"].as_str().unwrap().into(),
                parents: body["parents"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap().to_string())
                    .collect(),
                message: body["message"].as_str().unwrap().into(),
            };
            fake.commits.insert(sha.clone(), commit);
            reply(StatusCode::CREATED, json!({"sha": sha}))
        }
        ("GET", "pulls") => {
            let query = uri.query().unwrap_or_default();
            let head = urlencoding::decode(
                query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("head="))
                    .unwrap_or_default(),
            )
            .unwrap()
            .into_owned();
            let owner = REPO.split('/').next().unwrap();
            let open: Vec<Value> = fake
                .pulls
                .iter()
                .filter(|p| format!("{owner}:{}", p.head) == head)
                .map(|p| {
                    json!({
                        "number": p.number,
                        "html_url": format!("https://github.com/{REPO}/pull/{}", p.number),
                        "base": {"ref": p.base},
                    })
                })
                .collect();
            reply(StatusCode::OK, json!(open))
        }
        ("POST", "pulls") if fake.fail_pulls => reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message": format!("upstream rejected bearer {TOKEN}")}),
        ),
        ("POST", "pulls") => {
            let number = 100 + fake.pulls.len() as i64;
            fake.pulls.push(FakePull {
                number,
                head: body["head"].as_str().unwrap().into(),
                base: body["base"].as_str().unwrap().into(),
                title: body["title"].as_str().unwrap().into(),
                body: body["body"].as_str().unwrap().into(),
                draft: body["draft"].as_bool().unwrap(),
            });
            reply(
                StatusCode::CREATED,
                json!({
                    "number": number,
                    "html_url": format!("https://github.com/{REPO}/pull/{number}"),
                    "base": {"ref": body["base"]},
                }),
            )
        }
        _ => not_found(),
    }
}

async fn spawn_github(fake: Shared) -> String {
    let app = Router::new().fallback(any(github)).with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

// ---------------------------------------------------------------- Slack

#[derive(Default)]
struct RecordingSlack {
    posts: Mutex<Vec<(String, Option<String>, String)>>,
}

#[async_trait::async_trait]
impl SlackSender for RecordingSlack {
    async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError> {
        let mut posts = self.posts.lock().unwrap();
        posts.push((channel.into(), thread_ts.map(str::to_string), text.into()));
        Ok(Some(ExternalRef::Slack {
            channel_id: channel.into(),
            ts: format!("1759500001.{:06}", posts.len()),
        }))
    }

    async fn update_message(&self, _: &str, _: &str, _: &str) -> Result<(), SlackError> {
        Ok(())
    }
}

// ---------------------------------------------------------------- harness

struct Tenant {
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    member_id: MemberId,
}

struct Harness {
    store: Arc<dyn Store>,
    state: AppState,
    fake: Shared,
    slack: Arc<RecordingSlack>,
    base_sha: String,
    a: Tenant,
    b: Tenant,
}

const GREETING: &str = "line one\nhello\nline three\n";

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "pi".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("change".into()),
        })
        .await
        .unwrap();
    Tenant {
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        member_id: member.id,
    }
}

async fn harness() -> Harness {
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
    let a = tenant(store.as_ref(), "soundcheck").await;
    let b = tenant(store.as_ref(), "other").await;

    let fake: Shared = Arc::default();
    let base_sha = {
        let mut f = fake.lock().unwrap();
        let root = f.commit_files(&[("README.md", "readme\n")], vec![]);
        let base = f.commit_files(
            &[("README.md", "readme\n"), ("src/greeting.txt", GREETING)],
            vec![root],
        );
        f.refs.insert("dev".into(), base.clone());
        base
    };
    let github_base = spawn_github(fake.clone()).await;

    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.attach_github_sender(Arc::new(GithubApiClient::with_base_url(
        TOKEN.into(),
        github_base,
    )));
    let slack = Arc::new(RecordingSlack::default());
    state.attach_slack_sender(slack.clone());
    std::mem::forget(dir);
    Harness {
        store,
        state,
        fake,
        slack,
        base_sha,
        a,
        b,
    }
}

impl Harness {
    async fn allow(&self, ws: WorkspaceId, surface: EgressSurface, selector: &str) {
        self.store
            .allow_egress_target(NewEgressTarget {
                workspace_id: ws,
                surface,
                selector: selector.into(),
            })
            .await
            .unwrap();
    }

    /// Bless the branch surface (into `dev`) and the Slack channel for
    /// tenant A.
    async fn allow_change(&self) {
        self.allow(
            self.a.workspace_id,
            EgressSurface::GithubBranch,
            &format!("{REPO}@dev"),
        )
        .await;
        self.allow(self.a.workspace_id, EgressSurface::Slack, CHANNEL)
            .await;
    }

    async fn route(&self, t: &Tenant, result: &Value, log_id: i64) {
        self.store
            .set_thread_result(t.thread_id, t.member_id, result)
            .await
            .unwrap();
        notification_router::route_event(
            &self.state,
            log_id,
            &Event::ThreadResultSet {
                occurred_at: Utc::now(),
                workspace_id: t.workspace_id,
                channel_id: t.channel_id,
                thread_id: t.thread_id,
                produced_by: t.member_id,
            },
        )
        .await
        .unwrap();
    }

    /// Drain the queue. A change's Slack reply that was claimed before its
    /// branch delivery finished is deferred a few seconds; wait it out.
    async fn drain(&self) {
        for _ in 0..3 {
            let stats = egress_worker::sweep_once(&self.state).await;
            if stats.deferred == 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5_200)).await;
        }
    }

    async fn deliveries(&self, t: &Tenant) -> Vec<ResultDelivery> {
        self.store
            .list_result_deliveries(t.thread_id)
            .await
            .unwrap()
    }

    async fn delivery(&self, t: &Tenant, surface: &str) -> ResultDelivery {
        self.deliveries(t)
            .await
            .into_iter()
            .find(|d| d.surface == surface)
            .unwrap_or_else(|| panic!("no {surface} delivery"))
    }

    fn slack_posts(&self) -> Vec<(String, Option<String>, String)> {
        self.slack.posts.lock().unwrap().clone()
    }
}

const DIFF: &str = "diff --git a/src/greeting.txt b/src/greeting.txt\nindex 1111111..2222222 100644\n--- a/src/greeting.txt\n+++ b/src/greeting.txt\n@@ -1,3 +1,3 @@\n line one\n-hello\n+hello, world\n line three\n";

fn change(status: &str, base_sha: &str, branch: &str, diff: &str) -> Value {
    change_into(status, base_sha, branch, branch, "dev", diff)
}

/// A change whose result names `result_branch` and whose target names
/// `branch` into `base`.
fn change_into(
    status: &str,
    base_sha: &str,
    result_branch: &str,
    branch: &str,
    base: &str,
    diff: &str,
) -> Value {
    json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": PI_CHANGE_RESULT_KIND,
        "status": status,
        "base_sha": base_sha,
        "branch": result_branch,
        "diff": diff,
        "title": "Greet the world",
        "summary": "Changes the greeting. cc @beatgig/eng",
        "deliver_to": [
            {"surface": "slack", "channel": CHANNEL, "thread_ts": THREAD_TS},
            {"surface": "github_branch", "repo": REPO, "branch": branch, "base": base},
        ],
    })
}

// ---------------------------------------------------------------- tests

#[tokio::test]
async fn a_change_creates_the_branch_commits_once_and_opens_one_draft_pr() {
    let h = harness().await;
    h.allow_change().await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;

    let (head, commit, pull) = {
        let fake = h.fake.lock().unwrap();
        let head = fake
            .refs
            .get(BRANCH)
            .expect("the branch was created")
            .clone();
        let commit = fake.commits[&head].clone();
        assert_eq!(commit.parents, vec![h.base_sha.clone()]);
        assert_eq!(
            fake.file(&head, "src/greeting.txt").as_deref(),
            Some("line one\nhello, world\nline three\n")
        );
        assert_eq!(fake.file(&head, "README.md").as_deref(), Some("readme\n"));
        assert!(commit.message.starts_with("Greet the world\n"));
        assert!(commit.message.contains("Maidan-Change: "));
        assert_eq!(fake.pulls.len(), 1);
        (head, commit, fake.pulls[0].clone())
    };
    assert!(pull.draft, "the pull request opens as a draft");
    assert_eq!((pull.head.as_str(), pull.base.as_str()), (BRANCH, "dev"));
    assert_eq!(pull.title, "Greet the world");
    assert!(
        pull.body.contains("`@beatgig/eng`"),
        "mentions in the summary are defused: {}",
        pull.body
    );
    assert!(
        h.fake
            .lock()
            .unwrap()
            .writes()
            .iter()
            .all(|r| !r.contains("/issues/")
                && !r.contains("/reviews")
                && !r.contains("requested_reviewers")),
        "no comment, review or review request"
    );

    let branch_row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(branch_row.status, status::DELIVERED);
    assert_eq!(branch_row.selector, format!("{REPO}@{BRANCH}"));
    assert_eq!(
        branch_row.external_ref.as_deref(),
        Some(format!("{head}#{}", pull.number).as_str())
    );

    let posts = h.slack_posts();
    assert_eq!(posts.len(), 1, "{posts:?}");
    let (channel, thread_ts, text) = &posts[0];
    assert_eq!(channel, CHANNEL);
    assert_eq!(
        thread_ts.as_deref(),
        Some(THREAD_TS),
        "the reply is threaded"
    );
    assert!(text.contains(&head), "the reply names the commit: {text}");
    assert!(
        text.contains(&format!("https://github.com/{REPO}/pull/{}", pull.number)),
        "the reply links the PR: {text}"
    );
    assert_eq!(
        h.delivery(&h.a, "slack").await.selector,
        format!("{CHANNEL}/{THREAD_TS}")
    );

    // A retried delivery (operator replay) finds its own commit and the open
    // pull request: no second commit, no second pull request.
    let commits_before = h.fake.lock().unwrap().commits.len();
    let replayed = maidan_store::replay_result_delivery(
        h.store.as_ref(),
        h.a.workspace_id,
        h.a.thread_id,
        branch_row.id,
        String::new(),
    )
    .await
    .unwrap();
    assert!(matches!(
        replayed,
        Some(maidan_store::ResultDeliveryReplay::Enqueued(_))
    ));
    h.drain().await;
    {
        let fake = h.fake.lock().unwrap();
        assert_eq!(fake.commits.len(), commits_before, "no second commit");
        assert_eq!(fake.pulls.len(), 1, "no second pull request");
        assert_eq!(fake.refs[BRANCH], head);
        assert_eq!(fake.commits[&head].message, commit.message);
    }
    let again = h.delivery(&h.a, "github_branch").await;
    assert_eq!(again.status, status::DELIVERED);
    assert_eq!(again.external_ref, branch_row.external_ref);
}

#[tokio::test]
async fn a_branch_whose_head_is_not_base_sha_is_refused_and_said_in_slack() {
    let h = harness().await;
    h.allow_change().await;
    let elsewhere = {
        let mut f = h.fake.lock().unwrap();
        let sha = f.commit_files(
            &[("src/greeting.txt", "moved on\n")],
            vec![h.base_sha.clone()],
        );
        f.refs.insert(BRANCH.into(), sha.clone());
        sha
    };
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;

    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::SKIPPED);
    let reason = row.last_error.unwrap();
    assert!(reason.contains("not the result's base_sha"), "{reason}");
    let fake = h.fake.lock().unwrap();
    assert!(fake.writes().is_empty(), "{:?}", fake.writes());
    assert_eq!(fake.refs[BRANCH], elsewhere);
    drop(fake);
    let posts = h.slack_posts();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0].2.contains("Nothing was committed"),
        "{}",
        posts[0].2
    );
    assert_eq!(posts[0].1.as_deref(), Some(THREAD_TS));
}

#[tokio::test]
async fn a_result_for_another_branch_is_refused_without_touching_github() {
    let h = harness().await;
    h.allow_change().await;
    h.route(
        &h.a,
        &change_into(
            "changed",
            &h.base_sha,
            "feature/agent-other-9f9f",
            BRANCH,
            "dev",
            DIFF,
        ),
        1,
    )
    .await;
    h.drain().await;

    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::SKIPPED);
    assert!(
        row.last_error
            .as_deref()
            .unwrap()
            .contains("not the target branch"),
        "{:?}",
        row.last_error
    );
    assert!(h.fake.lock().unwrap().requests.is_empty());
}

#[tokio::test]
async fn a_diff_that_does_not_apply_is_refused_and_leaves_no_branch() {
    let h = harness().await;
    h.allow_change().await;
    let stale = DIFF.replace("-hello\n", "-goodbye\n");
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, &stale), 1)
        .await;
    h.drain().await;

    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::SKIPPED);
    let reason = row.last_error.unwrap();
    assert!(
        reason.contains("does not apply") && reason.contains("src/greeting.txt"),
        "{reason}"
    );
    let fake = h.fake.lock().unwrap();
    assert!(fake.writes().is_empty(), "{:?}", fake.writes());
    assert!(!fake.refs.contains_key(BRANCH));
}

#[tokio::test]
async fn a_non_changed_status_is_recorded_and_answered_in_slack_with_no_github_write() {
    for status_word in ["no_change", "seat_error", "content_blocked"] {
        let h = harness().await;
        h.allow_change().await;
        h.route(&h.a, &change(status_word, &h.base_sha, BRANCH, ""), 1)
            .await;
        h.drain().await;

        let row = h.delivery(&h.a, "github_branch").await;
        assert_eq!(row.status, status::SKIPPED, "{status_word}");
        assert!(
            row.last_error.as_deref().unwrap().contains(status_word),
            "{:?}",
            row.last_error
        );
        assert!(
            h.fake.lock().unwrap().requests.is_empty(),
            "{status_word}: no GitHub call at all"
        );
        let posts = h.slack_posts();
        assert_eq!(posts.len(), 1, "{status_word}");
        assert_eq!(posts[0].1.as_deref(), Some(THREAD_TS));
        assert!(posts[0].2.contains(status_word), "{}", posts[0].2);
        assert!(
            posts[0].2.contains("nothing was committed"),
            "{}",
            posts[0].2
        );
    }
}

#[tokio::test]
async fn an_unknown_surface_beside_a_change_is_still_a_recorded_skip() {
    let h = harness().await;
    h.allow_change().await;
    let mut result = change("changed", &h.base_sha, BRANCH, DIFF);
    result["deliver_to"]
        .as_array_mut()
        .unwrap()
        .push(json!({"surface": "gitlab_branch", "repo": REPO}));
    h.route(&h.a, &result, 1).await;
    h.drain().await;

    let unknown = h.delivery(&h.a, "gitlab_branch").await;
    assert_eq!(unknown.status, status::SKIPPED);
    assert!(unknown.last_error.unwrap().contains("unknown surface"));
    assert_eq!(
        h.delivery(&h.a, "github_branch").await.status,
        status::DELIVERED,
        "the unknown target does not sink the others"
    );
}

#[tokio::test]
async fn one_workspace_cannot_deliver_to_another_workspaces_allowed_repo() {
    let h = harness().await;
    // Tenant A blesses the repository and the channel; tenant B blesses
    // nothing and aims at both.
    h.allow_change().await;
    h.route(&h.b, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;

    for row in h.deliveries(&h.b).await {
        assert_eq!(row.status, status::SKIPPED, "{row:?}");
        assert_eq!(
            row.last_error.as_deref(),
            Some("target not in the workspace egress allowlist")
        );
    }
    assert!(h.fake.lock().unwrap().requests.is_empty());
    assert!(h.slack_posts().is_empty());
}

#[tokio::test]
async fn a_repo_allowed_for_comments_is_not_branch_writable() {
    let h = harness().await;
    h.allow(h.a.workspace_id, EgressSurface::Github, REPO).await;
    h.allow(h.a.workspace_id, EgressSurface::Slack, CHANNEL)
        .await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;

    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::SKIPPED);
    assert_eq!(
        row.last_error.as_deref(),
        Some("target not in the workspace egress allowlist")
    );
    assert!(h.fake.lock().unwrap().requests.is_empty());
    let posts = h.slack_posts();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0].2.contains("Nothing was committed"),
        "{}",
        posts[0].2
    );
}

#[tokio::test]
async fn a_review_result_with_a_slack_thread_ts_replies_in_that_thread() {
    let h = harness().await;
    h.allow(h.a.workspace_id, EgressSurface::Slack, CHANNEL)
        .await;
    let result = json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": "example.review.result/1",
        "status": "reviewed",
        "summary": "2 findings",
        "deliver_to": [
            {"surface": "slack", "channel": CHANNEL, "thread_ts": THREAD_TS},
            {"surface": "slack", "channel": CHANNEL},
        ],
    });
    h.route(&h.a, &result, 1).await;
    h.drain().await;

    let mut posts = h.slack_posts();
    posts.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(posts.len(), 2, "{posts:?}");
    assert_eq!(
        posts[0].1, None,
        "a target without thread_ts posts top-level"
    );
    assert_eq!(posts[1].1.as_deref(), Some(THREAD_TS));
    assert!(posts[1].2.contains("2 findings"));
    let target = EgressTarget::Slack {
        channel_id: CHANNEL.into(),
        thread_ts: Some(THREAD_TS.into()),
    };
    assert_eq!(
        h.store
            .get_result_delivery(h.a.thread_id, &target)
            .await
            .unwrap()
            .unwrap()
            .status,
        status::DELIVERED
    );
}

/// Route a change with this target and assert it was refused before any
/// GitHub call, with `why` in the recorded reason and in the Slack reply.
async fn assert_refused_without_github(h: &Harness, branch: &str, base: &str, why: &str) {
    h.route(
        &h.a,
        &change_into("changed", &h.base_sha, branch, branch, base, DIFF),
        1,
    )
    .await;
    h.drain().await;
    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::SKIPPED, "{branch} -> {base}");
    let reason = row.last_error.unwrap_or_default();
    assert!(reason.contains(why), "{branch} -> {base}: {reason}");
    assert!(
        h.fake.lock().unwrap().requests.is_empty(),
        "{branch} -> {base}: no GitHub call"
    );
    let posts = h.slack_posts();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0].2.contains("Nothing was committed"),
        "{}",
        posts[0].2
    );
}

#[tokio::test]
async fn a_prod_base_is_refused_even_when_an_operator_tries_to_bless_it() {
    let h = harness().await;
    h.allow_change().await;
    assert!(
        h.store
            .allow_egress_target(NewEgressTarget {
                workspace_id: h.a.workspace_id,
                surface: EgressSurface::GithubBranch,
                selector: format!("{REPO}@prod"),
            })
            .await
            .is_err(),
        "`prod` cannot be blessed as a base"
    );
    assert_refused_without_github(&h, BRANCH, "prod", "never allowed").await;
}

#[tokio::test]
async fn a_protected_branch_is_never_written() {
    for branch in ["main", "prod"] {
        let h = harness().await;
        h.allow_change().await;
        assert_refused_without_github(&h, branch, "dev", "protected").await;
    }
}

#[tokio::test]
async fn a_branch_without_the_agent_prefix_is_refused() {
    let h = harness().await;
    h.allow_change().await;
    assert_refused_without_github(&h, "feature/fix-greeting", "dev", "does not match").await;
}

#[tokio::test]
async fn a_base_outside_the_repos_allowlist_is_refused() {
    let h = harness().await;
    // `dev` is blessed; `staging` is not (and is protected as a branch, not
    // as a base, so only the allowlist stops it).
    h.allow_change().await;
    assert_refused_without_github(
        &h,
        BRANCH,
        "staging",
        "not in the workspace egress allowlist",
    )
    .await;
}

#[tokio::test]
async fn an_open_pull_request_into_prod_refuses_the_change_before_any_write() {
    let h = harness().await;
    h.allow_change().await;
    {
        let mut f = h.fake.lock().unwrap();
        f.refs.insert(BRANCH.into(), h.base_sha.clone());
        f.pulls.push(FakePull {
            number: 7,
            head: BRANCH.into(),
            base: "prod".into(),
            title: "someone else's".into(),
            body: String::new(),
            draft: false,
        });
    }
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;

    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::SKIPPED);
    let reason = row.last_error.unwrap();
    assert!(reason.contains("targets `prod`"), "{reason}");
    let fake = h.fake.lock().unwrap();
    assert!(fake.writes().is_empty(), "{:?}", fake.writes());
    assert_eq!(fake.refs[BRANCH], h.base_sha, "the branch did not move");
    assert_eq!(fake.pulls.len(), 1);
}

#[tokio::test]
async fn a_missing_title_or_summary_still_opens_the_draft_with_maidans_fallback() {
    let instructions = "!change bgv3 make the booking confirmation email name the venue and the start time instead of the generic greeting";
    for (drop_title, drop_summary) in [(true, false), (false, true), (true, true)] {
        let h = harness().await;
        h.allow_change().await;
        h.store
            .post_message_with_event(
                NewMessage {
                    thread_id: h.a.thread_id,
                    author_id: h.a.member_id,
                    body: instructions.into(),
                    metadata: json!({}),
                    content: None,
                },
                None,
            )
            .await
            .unwrap();
        let mut result = change("changed", &h.base_sha, BRANCH, DIFF);
        let fields = result.as_object_mut().unwrap();
        if drop_title {
            fields.remove("title");
        }
        if drop_summary {
            fields.remove("summary");
        }
        h.route(&h.a, &result, 1).await;
        h.drain().await;

        let case = format!("title dropped: {drop_title}, summary dropped: {drop_summary}");
        assert_eq!(
            h.delivery(&h.a, "github_branch").await.status,
            status::DELIVERED,
            "{case}"
        );
        let fake = h.fake.lock().unwrap();
        assert_eq!(fake.pulls.len(), 1, "{case}");
        let pull = &fake.pulls[0];
        let head = &fake.refs[BRANCH];
        if drop_title {
            assert!(pull.title.chars().count() <= 72, "{case}: {}", pull.title);
            assert!(
                instructions.starts_with(pull.title.trim_end_matches('…')),
                "{case}: {}",
                pull.title
            );
            assert!(
                fake.commits[head].message.starts_with(pull.title.as_str()),
                "{case}: the commit subject is the same title"
            );
        } else {
            assert_eq!(pull.title, "Greet the world", "{case}");
        }
        if drop_summary {
            assert!(pull.body.contains(head.as_str()), "{case}: {}", pull.body);
            assert!(
                pull.body.contains(&h.a.thread_id.0.to_string()),
                "{case}: {}",
                pull.body
            );
        } else {
            assert!(pull.body.starts_with("Changes the greeting."), "{case}");
        }
    }
}

#[tokio::test]
async fn a_rename_sent_as_a_delete_plus_an_add_moves_the_file() {
    let h = harness().await;
    h.allow_change().await;
    let diff = "diff --git a/src/greeting.txt b/src/greeting.txt\ndeleted file mode 100644\nindex 1111111..0000000\n--- a/src/greeting.txt\n+++ /dev/null\n@@ -1,3 +0,0 @@\n-line one\n-hello\n-line three\ndiff --git a/src/hello.txt b/src/hello.txt\nnew file mode 100644\nindex 0000000..2222222\n--- /dev/null\n+++ b/src/hello.txt\n@@ -0,0 +1,3 @@\n+line one\n+hello\n+line three\n";
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, diff), 1)
        .await;
    h.drain().await;

    assert_eq!(
        h.delivery(&h.a, "github_branch").await.status,
        status::DELIVERED
    );
    let fake = h.fake.lock().unwrap();
    let head = &fake.refs[BRANCH];
    assert_eq!(fake.file(head, "src/greeting.txt"), None);
    assert_eq!(fake.file(head, "src/hello.txt").as_deref(), Some(GREETING));
    assert_eq!(fake.file(head, "README.md").as_deref(), Some("readme\n"));
}

#[tokio::test]
async fn the_token_reaches_no_delivery_record_or_audit_row_when_github_fails() {
    let h = harness().await;
    h.allow_change().await;
    h.fake.lock().unwrap().fail_pulls = true;
    let mut result = change("changed", &h.base_sha, BRANCH, DIFF);
    // No Slack target: its reply would wait out the branch's retries.
    result["deliver_to"] = json!([
        {"surface": "github_branch", "repo": REPO, "branch": BRANCH, "base": "dev"},
    ]);
    h.route(&h.a, &result, 1).await;
    egress_worker::sweep_once(&h.state).await;

    let audit = h
        .store
        .list_audit_for_workspace(h.a.workspace_id, 100)
        .await
        .unwrap();
    let audit_text = serde_json::to_string(&audit).unwrap();
    assert!(
        audit.iter().any(
            |e| e.action == "result_delivery.attempt" && e.metadata.to_string().contains("500")
        ),
        "the failed attempt is audited with its error: {audit_text}"
    );
    assert!(!audit_text.contains(TOKEN), "{audit_text}");
    for row in h.deliveries(&h.a).await {
        assert!(!format!("{row:?}").contains(TOKEN), "{row:?}");
    }
}

#[tokio::test]
async fn a_change_reply_stops_waiting_on_a_branch_delivery_that_never_settles() {
    use maidan_server::result_delivery::{change_reply_at, ChangeReply, CHANGE_REPLY_MAX_WAIT};
    let h = harness().await;
    h.allow_change().await;
    // Routed but never sent: the branch row stays pending.
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    let produced_at = h
        .store
        .get_thread_result(h.a.thread_id)
        .await
        .unwrap()
        .unwrap()
        .produced_at;
    assert_eq!(
        h.delivery(&h.a, "github_branch").await.status,
        status::PENDING
    );

    let soon = produced_at + chrono::Duration::hours(1);
    assert!(matches!(
        change_reply_at(&h.state, h.a.thread_id, soon).await,
        ChangeReply::Waiting
    ));
    let late = produced_at + CHANGE_REPLY_MAX_WAIT + chrono::Duration::minutes(1);
    let ChangeReply::Ready(text) = change_reply_at(&h.state, h.a.thread_id, late).await else {
        panic!("the reply must stop waiting");
    };
    assert!(text.contains("Nothing has landed"), "{text}");
    assert!(text.contains(BRANCH), "{text}");
}
