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
    change_sweep, egress_worker,
    github::GithubApiClient,
    linear::{LinearError, LinearIssue, LinearSender, LinearState},
    notification_router,
    slack::SlackError,
    slack::SlackSender,
    AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    status, ChannelId, EgressSurface, EgressTarget, Event, EventKind, ExternalRef, MemberId,
    MemberKind, NewChannel, NewEgressTarget, NewMember, NewMessage, NewThread, NewWorkspace,
    ResultDelivery, ThreadId, ThreadState, WorkspaceId, CHANGE_RESULT_KIND, WAITER_RESULT_SCHEMA,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const REPO: &str = "example/app";
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
    labels: Vec<String>,
    /// Set by a test to report the pull request merged.
    merged_at: Option<String>,
    /// Set by a test to report the pull request closed without merging.
    closed_at: Option<String>,
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
    /// What `compare` reports a delivered base behind by; 2 when unset.
    behind: Option<i64>,
    /// Every `force: true` ref move, as `(branch, from, to)`.
    forced: Vec<(String, String, String)>,
    /// Every comment on a pull request, as `(number, body)`.
    pull_comments: Vec<(i64, String)>,
    /// Fail every comment with a 500.
    fail_comments: bool,
    /// The merge commit each merged pull request reports.
    merge_shas: HashMap<i64, String>,
    /// The commits on a pull request, when a test sets them.
    pull_commits: HashMap<i64, Vec<String>>,
    /// Reviews and review comments, as `(pull, "reviews" | "comments", item)`.
    reviews: Vec<(i64, &'static str, Value)>,
    /// The base branch's recent commits, as `(sha, message)`.
    base_commits: Vec<(String, String)>,
    /// Merged pull requests the change flow did not open, as GitHub lists them.
    merged_elsewhere: Vec<Value>,
    /// The `statusCheckRollup` contexts on a pull request's head, when a test
    /// sets them. A pull request without any answers the snapshot query with
    /// no commit, so the sweep records no checks for it.
    checks: HashMap<i64, Vec<Value>>,
    /// GitHub Actions job logs, served behind the redirect GitHub answers
    /// `actions/jobs/{id}/logs` with.
    job_logs: HashMap<i64, String>,
    /// A pull request's `mergeStateStatus`; `CLEAN` when unset.
    merge_states: HashMap<i64, String>,
    /// A pull request's review threads, as `(resolved, author)`.
    review_threads: HashMap<i64, Vec<(bool, String)>>,
    /// The status checks the base branch's protection requires; none when
    /// unset (the protection unreadable).
    required_checks: Option<Vec<String>>,
    /// Pull requests in the merge queue.
    queued: Vec<i64>,
    /// Why a pull request last left the merge queue.
    dequeued: HashMap<i64, String>,
    /// Commits already on the base branch, which `compare` reports behind.
    on_base: Vec<String>,
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
    if method == Method::POST && path == "/graphql" {
        return reply(StatusCode::OK, pull_snapshot(&fake, &body));
    }
    if let Some(job) = path.strip_prefix("/log-storage/") {
        return match fake.job_logs.get(&job.parse::<i64>().unwrap()) {
            Some(log) => log.clone().into_response(),
            None => reply(StatusCode::NOT_FOUND, json!({})),
        };
    }
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
            if body["force"] == json!(true) {
                assert!(
                    name.starts_with("feature/agent-"),
                    "only an agent branch is ever forced, not `{name}`"
                );
                if !fake.commits.contains_key(sha) {
                    return reply(StatusCode::UNPROCESSABLE_ENTITY, json!({}));
                }
                fake.forced.push((name.into(), current, sha.into()));
                fake.refs.insert(name.into(), sha.into());
                return reply(StatusCode::OK, json!({}));
            }
            assert_eq!(body["force"], json!(false), "force is always said");
            let fast_forward = fake
                .commits
                .get(sha)
                .is_some_and(|c| c.parents.contains(&current));
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
        ("GET", "pulls") if uri.query().unwrap_or_default().contains("state=closed") => {
            let merged: Vec<Value> = fake
                .pulls
                .iter()
                .filter(|p| p.merged_at.is_some())
                .map(|p| json!({"number": p.number, "title": p.title, "body": p.body, "merged_at": p.merged_at}))
                .chain(fake.merged_elsewhere.iter().cloned())
                .collect();
            reply(StatusCode::OK, json!(merged))
        }
        ("GET", "commits") => reply(
            StatusCode::OK,
            json!(fake
                .base_commits
                .iter()
                .map(|(sha, message)| json!({"sha": sha, "commit": {"message": message}}))
                .collect::<Vec<_>>()),
        ),
        ("GET", r) if r.starts_with("pulls/") && r.matches('/').count() == 2 => {
            let (number, list) = r["pulls/".len()..].split_once('/').unwrap();
            let number: i64 = number.parse().unwrap();
            let items: Vec<Value> = match list {
                "commits" => fake
                    .pull_commits
                    .get(&number)
                    .into_iter()
                    .flatten()
                    .map(|sha| json!({"sha": sha}))
                    .collect(),
                kind => fake
                    .reviews
                    .iter()
                    .filter(|(pull, k, _)| *pull == number && *k == kind)
                    .map(|(_, _, item)| item.clone())
                    .collect(),
            };
            reply(StatusCode::OK, json!(items))
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
                        "labels": p.labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(),
                        "title": p.title,
                        "body": p.body,
                    })
                })
                .collect();
            reply(StatusCode::OK, json!(open))
        }
        ("GET", r) if r.starts_with("actions/jobs/") && r.ends_with("/logs") => {
            let job = &r["actions/jobs/".len()..r.len() - "/logs".len()];
            (
                StatusCode::FOUND,
                [("location", format!("/log-storage/{job}"))],
            )
                .into_response()
        }
        ("GET", r) if r.starts_with("compare/") => {
            let head = r.rsplit("...").next().unwrap_or_default();
            let status = if fake.on_base.iter().any(|sha| sha == head) {
                "behind"
            } else {
                "diverged"
            };
            reply(
                StatusCode::OK,
                json!({"ahead_by": fake.behind.unwrap_or(2), "status": status}),
            )
        }
        ("GET", r) if r.starts_with("pulls/") => {
            let number: i64 = r["pulls/".len()..].parse().unwrap();
            match fake.pulls.iter().find(|p| p.number == number) {
                Some(p) => reply(
                    StatusCode::OK,
                    json!({
                        "number": p.number,
                        "head": {"ref": p.head, "sha": fake.refs.get(&p.head).cloned().unwrap_or_default(), "repo": {"full_name": REPO}},
                        "base": {"ref": p.base, "repo": {"full_name": REPO}},
                        "draft": p.draft,
                        "state": if p.merged_at.is_some() || p.closed_at.is_some() { "closed" } else { "open" },
                        "node_id": format!("PR_{}", p.number),
                        "merged_at": p.merged_at,
                        "closed_at": p.closed_at.clone().or_else(|| p.merged_at.clone()),
                        "merge_commit_sha": fake.merge_shas.get(&p.number),
                        "labels": p.labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(),
                    }),
                ),
                None => not_found(),
            }
        }
        ("PATCH", r) if r.starts_with("pulls/") => {
            let number: i64 = r["pulls/".len()..].parse().unwrap();
            match fake.pulls.iter_mut().find(|p| p.number == number) {
                Some(pull) => {
                    pull.body = body["body"].as_str().unwrap().into();
                    reply(StatusCode::OK, json!({"number": number}))
                }
                None => not_found(),
            }
        }
        ("POST", r) if r.starts_with("issues/") && r.ends_with("/comments") => {
            if fake.fail_comments {
                return reply(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"message": "down"}),
                );
            }
            let number: i64 = r["issues/".len()..r.len() - "/comments".len()]
                .parse()
                .unwrap();
            fake.pull_comments
                .push((number, body["body"].as_str().unwrap().into()));
            reply(StatusCode::CREATED, json!({"id": 1}))
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
                labels: Vec::new(),
                merged_at: None,
                closed_at: None,
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

/// The change sweep's GraphQL read of one pull request, answered from the
/// fake's state as GitHub would.
fn pull_snapshot(fake: &Fake, body: &Value) -> Value {
    assert!(
        body["query"]
            .as_str()
            .unwrap()
            .contains("statusCheckRollup"),
        "the only GraphQL query this fake answers is the sweep's read"
    );
    assert_eq!(
        format!(
            "{}/{}",
            body["variables"]["owner"].as_str().unwrap(),
            body["variables"]["name"].as_str().unwrap()
        ),
        REPO
    );
    let number = body["variables"]["number"].as_i64().unwrap();
    let Some(p) = fake.pulls.iter().find(|p| p.number == number) else {
        return json!({
            "data": {"repository": {"pullRequest": null}},
            "errors": [{"type": "NOT_FOUND", "message": "Could not resolve to a PullRequest"}],
        });
    };
    let head = fake.refs.get(&p.head).cloned().unwrap_or_default();
    let state = if p.merged_at.is_some() {
        "MERGED"
    } else if p.closed_at.is_some() {
        "CLOSED"
    } else {
        "OPEN"
    };
    let commits: Vec<Value> = fake
        .checks
        .get(&number)
        .map(|contexts| {
            json!({"commit": {"oid": head, "statusCheckRollup": {"contexts": {"nodes": contexts}}}})
        })
        .into_iter()
        .collect();
    json!({"data": {"repository": {"pullRequest": {
        "id": format!("PR_{number}"),
        "number": number,
        "state": state,
        "isDraft": p.draft,
        "isCrossRepository": false,
        "headRefName": p.head,
        "headRefOid": head,
        "baseRefName": p.base,
        "mergedAt": p.merged_at,
        "closedAt": p.closed_at.clone().or_else(|| p.merged_at.clone()),
        "mergeCommit": fake.merge_shas.get(&number).map(|oid| json!({"oid": oid})),
        "labels": {"nodes": p.labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>()},
        "commits": {"nodes": commits},
        "ahead": {"totalCount": if fake.on_base.contains(&head) { 0 } else { 1 }},
        "mergeStateStatus": fake.merge_states.get(&number).cloned().unwrap_or_else(|| "CLEAN".into()),
        "reviewThreads": {"nodes": fake.review_threads.get(&number).into_iter().flatten().enumerate().map(|(n, (resolved, author))| json!({
            "isResolved": resolved,
            "comments": {"nodes": [{
                "url": format!("https://github.com/{REPO}/pull/{number}#discussion_r{n}"),
                "author": {"login": author},
            }]},
        })).collect::<Vec<_>>()},
        "baseRef": {"branchProtectionRule": fake.required_checks.as_ref().map(|checks| json!({"requiredStatusCheckContexts": checks}))},
        "mergeQueueEntry": fake.queued.contains(&number).then(|| json!({"state": "QUEUED"})),
        "timelineItems": {"nodes": fake.dequeued.get(&number).map(|reason| json!({"__typename": "RemovedFromMergeQueueEvent", "reason": reason})).into_iter().collect::<Vec<_>>()},
    }}}})
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
    /// Every in-place edit, as `(channel, ts, text)`: a later revision of a
    /// result edits the reply it already posted.
    updates: Mutex<Vec<(String, String, String)>>,
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

    async fn update_message(&self, channel: &str, ts: &str, text: &str) -> Result<(), SlackError> {
        self.updates
            .lock()
            .unwrap()
            .push((channel.into(), ts.into(), text.into()));
        Ok(())
    }
}

// ---------------------------------------------------------------- Linear

const TEAM: &str = "6a1f1c4e-2f0b-4d5e-9a7b-0c1d2e3f4a5b";
const OTHER_TEAM: &str = "0b9e8d7c-6b5a-4f3e-8d2c-1b0a9f8e7d6c";
const ISSUE: &str = "ENG-7";
const ISSUE_ID: &str = "11111111-2222-4333-8444-555555555555";
const IN_REVIEW: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

/// Linear as the worker sees it: issues by identifier, and every write.
#[derive(Default)]
struct RecordingLinear {
    issues: Mutex<HashMap<String, LinearIssue>>,
    /// `(issue id, comment id, body)`, current bodies.
    comments: Mutex<Vec<(String, String, String)>>,
    updates: Mutex<u32>,
    moves: Mutex<Vec<(String, String)>>,
}

impl RecordingLinear {
    fn with_issue(team_id: &str) -> Self {
        let linear = Self::default();
        linear.issues.lock().unwrap().insert(
            ISSUE.into(),
            LinearIssue {
                id: ISSUE_ID.into(),
                identifier: ISSUE.into(),
                team_id: team_id.into(),
                states: vec![
                    LinearState {
                        id: "99999999-9999-4999-8999-999999999999".into(),
                        name: "Todo".into(),
                    },
                    LinearState {
                        id: IN_REVIEW.into(),
                        name: "In Review".into(),
                    },
                ],
            },
        );
        linear
    }

    fn comments(&self) -> Vec<(String, String, String)> {
        self.comments.lock().unwrap().clone()
    }

    fn moves(&self) -> Vec<(String, String)> {
        self.moves.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl LinearSender for RecordingLinear {
    async fn issue(&self, issue: &str) -> Result<LinearIssue, LinearError> {
        self.issues
            .lock()
            .unwrap()
            .get(issue)
            .cloned()
            .ok_or_else(|| LinearError::NotFound(format!("issue {issue}")))
    }

    async fn create_comment(&self, issue_id: &str, body: &str) -> Result<String, LinearError> {
        let mut comments = self.comments.lock().unwrap();
        let id = format!("cccccccc-0000-4000-8000-{:012}", comments.len() + 1);
        comments.push((issue_id.into(), id.clone(), body.into()));
        Ok(id)
    }

    async fn update_comment(&self, comment_id: &str, body: &str) -> Result<(), LinearError> {
        let mut comments = self.comments.lock().unwrap();
        let Some(comment) = comments.iter_mut().find(|c| c.1 == comment_id) else {
            return Err(LinearError::NotFound(format!("comment {comment_id}")));
        };
        comment.2 = body.into();
        *self.updates.lock().unwrap() += 1;
        Ok(())
    }

    async fn move_issue(&self, issue_id: &str, state_id: &str) -> Result<(), LinearError> {
        self.moves
            .lock()
            .unwrap()
            .push((issue_id.into(), state_id.into()));
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
    linear: Arc<RecordingLinear>,
    base_sha: String,
    a: Tenant,
    b: Tenant,
}

const GREETING: &str = "line one\nhello\nline three\n";
const BRIDGE: &str = "pub fn bridge() {}\n";

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "coder".into(),
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
            description: None,
            created_by: None,
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
    harness_with(RecordingLinear::with_issue(TEAM)).await
}

async fn harness_with(linear: RecordingLinear) -> Harness {
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
    let a = tenant(store.as_ref(), "change-flow").await;
    let b = tenant(store.as_ref(), "other").await;

    let fake: Shared = Arc::default();
    let base_sha = {
        let mut f = fake.lock().unwrap();
        let root = f.commit_files(&[("README.md", "readme\n")], vec![]);
        let base = f.commit_files(
            &[
                ("README.md", "readme\n"),
                ("src/greeting.txt", GREETING),
                ("src/bridge/x.rs", BRIDGE),
            ],
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
    state.attach_github_sender(Arc::new(
        GithubApiClient::with_base_url(TOKEN.into(), github_base).with_any_write_repo(),
    ));
    let slack = Arc::new(RecordingSlack::default());
    state.attach_slack_sender(slack.clone());
    let linear = Arc::new(linear);
    state.attach_linear_sender(linear.clone());
    std::mem::forget(dir);
    Harness {
        store,
        state,
        fake,
        slack,
        linear,
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

    /// The Slack reply as it reads now: the newest edit, else the newest
    /// post.
    fn slack_reply(&self) -> String {
        if let Some(edit) = self.slack.updates.lock().unwrap().last() {
            return edit.2.clone();
        }
        self.slack_posts()
            .last()
            .map(|p| p.2.clone())
            .unwrap_or_default()
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
        "result_kind": CHANGE_RESULT_KIND,
        "status": status,
        "base_sha": base_sha,
        "branch": result_branch,
        "diff": diff,
        "title": "Greet the world",
        "summary": "Changes the greeting. cc @example/eng",
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
        assert!(fake.forced.is_empty(), "a normal round never forces");
        assert_eq!(fake.pulls.len(), 1);
        (head, commit, fake.pulls[0].clone())
    };
    assert!(pull.draft, "the pull request opens as a draft");
    assert_eq!((pull.head.as_str(), pull.base.as_str()), (BRANCH, "dev"));
    assert_eq!(pull.title, "Greet the world");
    assert!(
        pull.body.contains("`@example/eng`"),
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
async fn a_seat_error_reply_names_its_cause_once_in_place_of_the_summary() {
    let h = harness().await;
    h.allow_change().await;
    let mut result = change("seat_error", &h.base_sha, BRANCH, "");
    result["error_detail"] =
        json!("pi gave up after 3 failed attempts\nthe seat timed out\nthe seat timed out");
    result["summary"] = json!("a summary the detail outranks");
    h.route(&h.a, &result, 1).await;
    h.drain().await;

    let posts = h.slack_posts();
    assert_eq!(posts.len(), 1, "one end notice");
    let body = &posts[0].2;
    assert!(body.contains("`seat_error`"), "{body}");
    assert!(
        body.contains("pi gave up after 3 failed attempts"),
        "{body}"
    );
    assert_eq!(body.matches("the seat timed out").count(), 1, "{body}");
    assert!(!body.contains("a summary the detail outranks"), "{body}");
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
            labels: Vec::new(),
            merged_at: None,
            closed_at: None,
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
    let instructions = "!change example-repo make the booking confirmation email name the venue and the start time instead of the generic greeting";
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

/// The producer's pull request template, with check output fenced and one
/// mention in prose.
const TEMPLATED_BODY: &str = "## What and why\nThe greeting names the world. Fixes ENG-1234.\n\n## Testing\n```\n$ cargo test -p greeting\ntest result: ok. 3 passed; 0 failed\n```\n\n## Risk\nLow. cc @example/eng\n\n## How to revert\nRevert the commit.\n\nPart of ENG-1200\n";

#[tokio::test]
async fn a_templated_body_passes_through_and_gets_no_template_warnings() {
    let h = harness().await;
    h.allow_change().await;
    let mut result = change("changed", &h.base_sha, BRANCH, DIFF);
    result["title"] = json!("[ENG-1234] Greet the world");
    result["summary"] = json!(TEMPLATED_BODY);
    h.route(&h.a, &result, 1).await;
    h.drain().await;

    {
        let fake = h.fake.lock().unwrap();
        let pull = &fake.pulls[0];
        assert_eq!(pull.title, "[ENG-1234] Greet the world");
        assert_eq!(
            pull.body,
            TEMPLATED_BODY.replace("@example/eng", "`@example/eng`"),
            "byte for byte, apart from the defused mention"
        );
    }
    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::DELIVERED);
    assert!(row.template_warnings.is_empty(), "{row:?}");
    let reply = h.slack_reply();
    assert!(reply.contains("Draft PR:"), "{reply}");
    assert!(!reply.contains("PR template"), "{reply}");
}

#[tokio::test]
async fn a_pull_request_missing_its_testing_section_and_issue_key_is_flagged_not_refused() {
    let h = harness().await;
    h.allow_change().await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;

    assert!(
        h.fake.lock().unwrap().pulls[0].draft,
        "the draft opens anyway"
    );
    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::DELIVERED);
    assert_eq!(row.last_error, None, "a warning is not a failure");
    assert_eq!(
        row.template_warnings,
        vec!["missing_testing", "missing_issue_key"]
    );
    let reply = h.slack_reply();
    assert!(
        reply.contains(
            "PR template: missing a Testing section and an issue key (like ENG-123) in the title or body."
        ),
        "{reply}"
    );

    let token = read_token(&h, h.a.workspace_id, h.a.member_id).await;
    let (code, rows) = get_json(
        &h,
        &token,
        &format!("/threads/{}/deliveries", h.a.thread_id.0),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let branch = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["surface"] == "github_branch")
        .unwrap();
    assert_eq!(
        branch["template_warnings"],
        json!(["missing_testing", "missing_issue_key"])
    );
    let slack = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["surface"] == "slack")
        .unwrap();
    assert!(slack.get("template_warnings").is_none(), "{slack}");

    // A later round whose result carries the template does not change the
    // open pull request's title or body, so its warnings stand.
    let mut templated = change("changed", &h.base_sha, BRANCH, DIFF);
    templated["title"] = json!("[ENG-1234] Greet the world");
    templated["summary"] = json!(TEMPLATED_BODY);
    h.route(&h.a, &templated, 2).await;
    h.drain().await;
    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::DELIVERED);
    assert_eq!(
        row.template_warnings,
        vec!["missing_testing", "missing_issue_key"]
    );

    // A person fixes the pull request itself; the next round reads it and
    // clears them.
    {
        let mut f = h.fake.lock().unwrap();
        f.pulls[0].title = "[ENG-1234] Greet the world".into();
        f.pulls[0].body = TEMPLATED_BODY.into();
    }
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 3)
        .await;
    h.drain().await;
    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::DELIVERED);
    assert!(row.template_warnings.is_empty(), "{row:?}");
    assert!(
        !h.slack_reply().contains("PR template"),
        "{}",
        h.slack_reply()
    );
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
        change_reply_at(&h.state, h.a.workspace_id, h.a.thread_id, soon).await,
        ChangeReply::Waiting
    ));
    let late = produced_at + CHANGE_REPLY_MAX_WAIT + chrono::Duration::minutes(1);
    let ChangeReply::Ready(text) =
        change_reply_at(&h.state, h.a.workspace_id, h.a.thread_id, late).await
    else {
        panic!("the reply must stop waiting");
    };
    assert!(text.contains("Nothing has landed"), "{text}");
    assert!(text.contains(BRANCH), "{text}");
}

// ---------------------------------------------------------------- Linear

/// A change whose result also comments on [`ISSUE`] in `team`.
fn change_to_linear(status: &str, base_sha: &str, team: &str, state: Option<&str>) -> Value {
    let mut result = change(status, base_sha, BRANCH, DIFF);
    let mut target = json!({"surface": "linear", "team": team, "issue": ISSUE});
    if let Some(state) = state {
        target["state"] = json!(state);
    }
    result["deliver_to"].as_array_mut().unwrap().push(target);
    result
}

/// A task whose attributes name `issue`, in `t`'s channel, claimed by
/// `t`'s member.
async fn linear_task(h: &Harness, t: &Tenant, issue: &str) -> ThreadId {
    let (thread, _) = h
        .store
        .create_thread_with_footprint(
            NewThread {
                channel_id: t.channel_id,
                parent_thread_id: None,
                title: Some("Name the venue".into()),
                description: None,
                created_by: Some(t.member_id),
            },
            None,
            Some(maidan_types::TaskAttributes {
                engine: Some(maidan_types::TaskEngine::Pi),
                linear_issue: Some(issue.into()),
                ..maidan_types::TaskAttributes::default()
            }),
            None,
        )
        .await
        .unwrap();
    h.store.claim_thread(thread.id, t.member_id).await.unwrap();
    thread.id
}

async fn admin_token(h: &Harness, t: &Tenant) -> String {
    let secret = maidan_auth::TokenSecret::generate();
    h.store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id: t.workspace_id,
            member_id: t.member_id,
            app_installation_id: None,
            token_hash: maidan_auth::hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![maidan_auth::capability::TOKEN_ADMIN.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn claim_comments_stay_off_until_the_workspace_turns_them_on_then_say_each_once() {
    let h = harness().await;
    h.allow(h.a.workspace_id, EgressSurface::Linear, TEAM).await;
    h.allow(h.b.workspace_id, EgressSurface::Linear, TEAM).await;
    let task = linear_task(&h, &h.a, ISSUE).await;
    linear_task(&h, &h.b, ISSUE).await;

    assert!(!h
        .store
        .linear_claim_comments(h.a.workspace_id)
        .await
        .unwrap());
    assert_eq!(change_sweep::sweep_once(&h.state).await.linear_notices, 0);
    h.drain().await;
    assert!(
        h.linear.comments().is_empty(),
        "off by default: nothing is said"
    );

    let token = admin_token(&h, &h.a).await;
    let response = tower::ServiceExt::oneshot(
        maidan_server::router(h.state.clone()),
        axum::http::Request::put(format!(
            "/workspaces/{}/linear-claim-comments",
            h.a.workspace_id.0
        ))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(r#"{"enabled": true}"#))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let read = read_token(&h, h.a.workspace_id, h.a.member_id).await;
    let (_, switch) = get_json(
        &h,
        &read,
        &format!("/workspaces/{}/linear-claim-comments", h.a.workspace_id.0),
    )
    .await;
    assert_eq!(switch, json!({"enabled": true}));

    assert_eq!(change_sweep::sweep_once(&h.state).await.linear_notices, 1);
    h.drain().await;
    let comments = h.linear.comments();
    assert_eq!(
        comments.len(),
        1,
        "tenant B's switch is still off: {comments:?}"
    );
    assert_eq!(comments[0].0, ISSUE_ID);
    assert!(
        comments[0]
            .2
            .starts_with(&format!("CLAIM: pi/{} claimed in Maidan by ", task.0)),
        "{}",
        comments[0].2
    );
    assert!(
        comments[0].2.ends_with(": Name the venue"),
        "{}",
        comments[0].2
    );
    assert_eq!(
        change_sweep::sweep_once(&h.state).await.linear_notices,
        0,
        "said once"
    );

    // It ends without a merge: one release.
    h.store
        .cancel_thread(
            task,
            h.a.member_id,
            "out of scope",
            maidan_types::CancelAuthority::Creator,
        )
        .await
        .unwrap();
    assert_eq!(change_sweep::sweep_once(&h.state).await.linear_notices, 1);
    h.drain().await;
    let comments = h.linear.comments();
    assert_eq!(comments.len(), 2, "{comments:?}");
    assert_eq!(
        comments[1].2,
        format!(
            "RELEASED: pi/{} ended without merging: Name the venue",
            task.0
        )
    );
    assert_eq!(change_sweep::sweep_once(&h.state).await.linear_notices, 0);
    h.drain().await;
    assert_eq!(h.linear.comments().len(), 2);
}

/// A Pi task past the round limit is handed to a session exactly once: its
/// claim ends, its engine becomes `session` (a Pi claim skips it), a
/// `ThreadHandoff` event, a thread message and a Slack line say so, and its
/// Linear issue gets the packet and then the release. Tenant B is untouched.
#[tokio::test]
async fn a_non_converging_pi_task_is_handed_to_a_session_once() {
    let h = harness().await;
    h.allow_change().await;
    h.allow(h.a.workspace_id, EgressSurface::Linear, TEAM).await;
    h.store
        .set_linear_claim_comments(
            h.a.workspace_id,
            true,
            maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(h.a.workspace_id),
                actor_id: Some(h.a.member_id),
                action: "linear.claim_comments.set".into(),
                target_kind: None,
                target_id: None,
                metadata: json!({}),
            },
        )
        .await
        .unwrap();
    let pi = maidan_types::TaskAttributes {
        engine: Some(maidan_types::TaskEngine::Pi),
        linear_issue: Some(ISSUE.into()),
        ..maidan_types::TaskAttributes::default()
    };
    h.store
        .set_thread_attributes(h.a.thread_id, pi.clone())
        .await
        .unwrap();
    h.store
        .set_thread_attributes(h.b.thread_id, pi)
        .await
        .unwrap();
    h.store
        .claim_thread(h.a.thread_id, h.a.member_id)
        .await
        .unwrap();
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;
    assert_eq!(
        change_sweep::sweep_once(&h.state).await.handed_off,
        0,
        "one round"
    );
    h.drain().await;
    let number = h.fake.lock().unwrap().pulls[0].number;
    for n in 2..=3u8 {
        h.store
            .record_change_round(maidan_types::NewChangeRound {
                workspace_id: h.a.workspace_id,
                thread_id: h.a.thread_id,
                repo: REPO.into(),
                pull_number: number,
                commit_sha: format!("{n}").repeat(40),
                base_sha: h.base_sha.clone(),
                base_behind: Some(1),
                diff_lines: 5,
                paths: Vec::new(),
                base_branch: Some("dev".into()),
                rebase: false,
                previous_head: None,
                linear_issue: None,
            })
            .await
            .unwrap();
    }

    assert_eq!(change_sweep::sweep_once(&h.state).await.handed_off, 1);
    h.drain().await;
    let thread = h.store.get_thread(h.a.thread_id).await.unwrap();
    assert_eq!(thread.assignee_id, None, "the claim ended");
    assert_eq!(thread.state, ThreadState::Open, "left open for a session");
    assert_eq!(
        h.store
            .get_thread_attributes(h.a.thread_id)
            .await
            .unwrap()
            .engine,
        Some(maidan_types::TaskEngine::Session)
    );
    assert!(
        h.store
            .claim_next_thread(
                h.a.channel_id,
                h.a.member_id,
                None,
                Some(maidan_types::TaskEngine::Pi)
            )
            .await
            .unwrap()
            .is_none(),
        "a Pi claim never takes it again"
    );
    let handoffs: Vec<_> = h
        .store
        .list_events_after(h.a.workspace_id, 0, 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == EventKind::ThreadHandoff)
        .collect();
    assert_eq!(handoffs.len(), 1);
    assert_eq!(handoffs[0].payload["rounds"], 3);
    assert_eq!(handoffs[0].payload["to_engine"], "session");

    let comments: Vec<String> = h.linear.comments().into_iter().map(|c| c.2).collect();
    assert_eq!(comments.len(), 3, "{comments:#?}");
    assert!(comments[0].starts_with("CLAIM: pi/"), "{}", comments[0]);
    assert!(comments[1].starts_with("HANDOFF: pi/"), "{}", comments[1]);
    for part in [
        "after 3 rounds without converging",
        &format!(
            "PR: https://github.com/{REPO}/pull/{number} at {}",
            "3".repeat(40)
        ),
        "- 3: 5 lines, base 1 behind",
        "Cost: $0.00",
        "Open findings: none reported",
        "Models: not reported",
    ] {
        assert!(
            comments[1].contains(part),
            "{part} missing from {}",
            comments[1]
        );
    }
    assert!(
        comments[2].starts_with(&format!(
            "RELEASED: pi/{} handed off to a session after 3 rounds",
            h.a.thread_id.0
        )),
        "{}",
        comments[2]
    );
    assert!(
        h.slack_posts()
            .iter()
            .any(|(channel, ts, text)| channel == CHANNEL
                && ts.as_deref() == Some(THREAD_TS)
                && text.contains("Handed to a session")),
        "{:?}",
        h.slack_posts()
    );
    let token = read_token(&h, h.a.workspace_id, h.a.member_id).await;
    let (_, metrics) = get_json(
        &h,
        &token,
        &format!("/workspaces/{}/swarm-metrics", h.a.workspace_id.0),
    )
    .await;
    assert_eq!(metrics["tasks"][0]["handed_off"], true);
    assert_eq!(metrics["repos"][0]["misrouted"], 1);

    let again = change_sweep::sweep_once(&h.state).await;
    assert_eq!((again.handed_off, again.linear_notices), (0, 0));
    h.drain().await;
    assert_eq!(h.linear.comments().len(), 3);
    assert_eq!(
        h.store
            .get_thread_attributes(h.b.thread_id)
            .await
            .unwrap()
            .engine,
        Some(maidan_types::TaskEngine::Pi),
        "tenant B is untouched"
    );
}

/// A session's pull request, registered on its task, is tracked like the
/// change flow's: time to ready runs from the task's `CLAIM:` comment to the
/// `ready-to-merge` label, and the `linear_issue` filter counts it.
#[tokio::test]
async fn a_registered_session_pull_counts_from_its_claim_to_the_ready_label() {
    let h = harness().await;
    h.allow_change().await;
    h.allow(h.a.workspace_id, EgressSurface::Linear, TEAM).await;
    h.store
        .set_linear_claim_comments(
            h.a.workspace_id,
            true,
            maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(h.a.workspace_id),
                actor_id: Some(h.a.member_id),
                action: "linear.claim_comments.set".into(),
                target_kind: None,
                target_id: None,
                metadata: json!({}),
            },
        )
        .await
        .unwrap();
    h.store
        .set_thread_attributes(
            h.a.thread_id,
            maidan_types::TaskAttributes {
                engine: Some(maidan_types::TaskEngine::Session),
                linear_issue: Some(ISSUE.into()),
                ..maidan_types::TaskAttributes::default()
            },
        )
        .await
        .unwrap();
    h.store
        .claim_thread(h.a.thread_id, h.a.member_id)
        .await
        .unwrap();
    change_sweep::sweep_once(&h.state).await;
    h.drain().await;
    assert!(h.linear.comments()[0].2.starts_with("CLAIM: session/"));
    h.fake.lock().unwrap().pulls.push(FakePull {
        number: 77,
        head: "session/name-the-venue".into(),
        base: "dev".into(),
        title: "Name the venue".into(),
        body: String::new(),
        draft: false,
        labels: Vec::new(),
        merged_at: None,
        closed_at: None,
    });

    let secret = maidan_auth::TokenSecret::generate();
    h.store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id: h.a.workspace_id,
            member_id: h.a.member_id,
            app_installation_id: None,
            token_hash: maidan_auth::hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![maidan_auth::capability::WORKSPACE_WRITE.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let register = |repo: &str| {
        axum::http::Request::put(format!("/threads/{}/pull", h.a.thread_id.0))
            .header("authorization", format!("Bearer {}", secret.as_str()))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({"repo": repo, "pull_number": 77}).to_string(),
            ))
            .unwrap()
    };
    let send =
        |request| tower::ServiceExt::oneshot(maidan_server::router(h.state.clone()), request);
    assert_eq!(
        send(register("other/repo")).await.unwrap().status(),
        StatusCode::FORBIDDEN,
        "only a blessed repository"
    );
    for _ in 0..2 {
        assert_eq!(
            send(register(REPO)).await.unwrap().status(),
            StatusCode::OK,
            "registering again answers the same"
        );
    }

    change_sweep::sweep_once(&h.state).await;
    let pulls = h
        .store
        .list_change_pulls(h.a.workspace_id, None, None, None, None, None)
        .await
        .unwrap();
    let pull = pulls.iter().find(|p| p.pull_number == 77).unwrap();
    assert_eq!(pull.ready_at, None, "no label yet");
    h.fake.lock().unwrap().pulls.last_mut().unwrap().labels = vec!["ready-to-merge".into()];
    change_sweep::sweep_once(&h.state).await;

    let token = read_token(&h, h.a.workspace_id, h.a.member_id).await;
    let (_, metrics) = get_json(
        &h,
        &token,
        &format!(
            "/workspaces/{}/swarm-metrics?linear_issue={ISSUE}",
            h.a.workspace_id.0
        ),
    )
    .await;
    let tasks = metrics["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1, "{metrics}");
    assert_eq!(tasks[0]["pull_number"], 77);
    assert!(tasks[0]["time_to_ready_secs"].is_i64(), "{}", tasks[0]);
    assert_eq!(metrics["repos"][0]["by_engine"][0]["engine"], "session");
}

/// A bearer token for `member` in `workspace` holding exactly `caps`.
async fn token_with(
    h: &Harness,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    caps: &[&str],
) -> String {
    let secret = maidan_auth::TokenSecret::generate();
    h.store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: maidan_auth::hash_secret(secret.as_str()),
            label: None,
            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn call_tool(h: &Harness, token: &str, name: &str, arguments: Value) -> Value {
    let response = tower::ServiceExt::oneshot(
        maidan_server::router(h.state.clone()),
        axum::http::Request::post("/mcp")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": { "name": name, "arguments": arguments }
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await
    .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn tool_json(res: &Value) -> Value {
    let text = res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no tool result: {res}"));
    serde_json::from_str(text).unwrap()
}

/// A session registers its pull request and reads the swarm metrics over
/// MCP, with the route's rules: `workspace:write` and thread access to
/// register, a blessed repository, `workspace:read` on the caller's own
/// workspace to read, and the GitHub refresh first, so a merge counts at once.
#[test]
fn a_session_registers_its_pull_and_reads_the_metrics_over_mcp() {
    // An MCP call through the router in an unoptimized build needs about
    // 1.8 MiB of stack here, so on a 2 MiB test thread it overflowed on CI.
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(session_pull_over_mcp())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn session_pull_over_mcp() {
    let h = harness().await;
    h.allow_change().await;
    // Tenant B blesses the same repository, so only thread access can refuse it.
    h.allow(
        h.b.workspace_id,
        EgressSurface::GithubBranch,
        &format!("{REPO}@dev"),
    )
    .await;
    h.fake.lock().unwrap().pulls.push(FakePull {
        number: 78,
        head: "session/rename".into(),
        base: "dev".into(),
        title: "Rename".into(),
        body: String::new(),
        draft: false,
        labels: Vec::new(),
        merged_at: None,
        closed_at: None,
    });
    let writer = token_with(
        &h,
        h.a.workspace_id,
        h.a.member_id,
        &[
            maidan_auth::capability::WORKSPACE_READ,
            maidan_auth::capability::WORKSPACE_WRITE,
        ],
    )
    .await;
    let reader = read_token(&h, h.a.workspace_id, h.a.member_id).await;
    let other = token_with(
        &h,
        h.b.workspace_id,
        h.b.member_id,
        &[
            maidan_auth::capability::WORKSPACE_READ,
            maidan_auth::capability::WORKSPACE_WRITE,
        ],
    )
    .await;
    let register = json!({ "thread_id": h.a.thread_id.0, "repo": REPO, "pull_number": 78 });

    let refused = call_tool(&h, &reader, "register_thread_pull", register.clone()).await;
    assert!(
        refused.get("error").is_some(),
        "workspace:read cannot register: {refused}"
    );
    let refused = call_tool(&h, &other, "register_thread_pull", register.clone()).await;
    assert_eq!(
        refused["error"]["code"], -32003,
        "another tenant's thread: {refused}"
    );
    let refused = call_tool(
        &h,
        &writer,
        "register_thread_pull",
        json!({ "thread_id": h.a.thread_id.0, "repo": "other/repo", "pull_number": 78 }),
    )
    .await;
    assert_eq!(
        refused["error"]["code"], -32003,
        "an unblessed repository: {refused}"
    );
    let refused = call_tool(
        &h,
        &writer,
        "register_thread_pull",
        json!({ "thread_id": h.a.thread_id.0, "repo": REPO, "pull_number": 0 }),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");

    let registered = tool_json(&call_tool(&h, &writer, "register_thread_pull", register).await);
    assert_eq!(registered["repo"], REPO);
    assert_eq!(registered["pull_number"], 78);
    assert!(
        h.store
            .list_change_pulls(h.b.workspace_id, None, None, None, None, None)
            .await
            .unwrap()
            .is_empty(),
        "tenant B tracks nothing"
    );

    h.fake.lock().unwrap().pulls.last_mut().unwrap().merged_at =
        Some("2026-10-20T12:00:00Z".into());
    let metrics = tool_json(
        &call_tool(
            &h,
            &reader,
            "get_swarm_metrics",
            json!({ "workspace_id": h.a.workspace_id.0 }),
        )
        .await,
    );
    let tasks = metrics["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1, "{metrics}");
    assert_eq!(tasks[0]["pull_number"], 78);
    assert_eq!(
        tasks[0]["merged_at"], "2026-10-20T12:00:00Z",
        "the read refreshed the pull request from GitHub first"
    );
    let (status, rest) = get_json(
        &h,
        &reader,
        &format!("/workspaces/{}/swarm-metrics", h.a.workspace_id.0),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        rest["tasks"], metrics["tasks"],
        "the twin answers as the route"
    );

    let refused = call_tool(
        &h,
        &other,
        "get_swarm_metrics",
        json!({ "workspace_id": h.a.workspace_id.0 }),
    )
    .await;
    assert_eq!(
        refused["error"]["code"], -32003,
        "another tenant's metrics: {refused}"
    );
    let refused = call_tool(
        &h,
        &reader,
        "get_swarm_metrics",
        json!({ "workspace_id": h.a.workspace_id.0, "linear_issue": "not a key" }),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
}

#[tokio::test]
async fn a_claim_comment_on_an_issue_in_an_unblessed_team_is_skipped() {
    let h = harness().await;
    h.store
        .set_linear_claim_comments(
            h.a.workspace_id,
            true,
            maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(h.a.workspace_id),
                actor_id: Some(h.a.member_id),
                action: "linear.claim_comments.set".into(),
                target_kind: None,
                target_id: None,
                metadata: json!({}),
            },
        )
        .await
        .unwrap();
    linear_task(&h, &h.a, ISSUE).await;
    assert_eq!(change_sweep::sweep_once(&h.state).await.linear_notices, 0);
    h.drain().await;
    assert!(h.linear.comments().is_empty());
    assert_eq!(
        change_sweep::sweep_once(&h.state).await.linear_notices,
        0,
        "recorded as skipped, not tried again"
    );
}

#[tokio::test]
async fn a_change_comments_on_its_linear_issue_with_the_draft_pr_and_moves_it() {
    let h = harness().await;
    h.allow_change().await;
    h.allow(h.a.workspace_id, EgressSurface::Linear, TEAM).await;
    h.route(
        &h.a,
        &change_to_linear("changed", &h.base_sha, TEAM, Some("In Review")),
        1,
    )
    .await;
    h.drain().await;

    let comments = h.linear.comments();
    assert_eq!(comments.len(), 1, "{comments:?}");
    let (issue_id, comment_id, body) = &comments[0];
    assert_eq!(
        issue_id, ISSUE_ID,
        "the comment goes on the issue by its id"
    );
    assert!(
        body.contains(&format!("Draft PR: https://github.com/{REPO}/pull/")),
        "the comment carries the pull request the branch delivery opened: {body}"
    );
    assert_eq!(
        h.linear.moves(),
        vec![(ISSUE_ID.to_string(), IN_REVIEW.to_string())]
    );

    let row = h.delivery(&h.a, "linear").await;
    assert_eq!(row.status, status::DELIVERED, "{:?}", row.last_error);
    assert_eq!(
        row.reference(),
        Some(ExternalRef::Linear {
            comment_id: comment_id.clone()
        })
    );
}

#[tokio::test]
async fn an_unblessed_linear_team_gets_no_comment() {
    let h = harness().await;
    h.allow_change().await;
    h.route(
        &h.a,
        &change_to_linear("changed", &h.base_sha, TEAM, None),
        1,
    )
    .await;
    h.drain().await;

    let row = h.delivery(&h.a, "linear").await;
    assert_eq!(row.status, status::SKIPPED);
    assert!(
        row.last_error.as_deref().unwrap().contains("allowlist"),
        "{:?}",
        row.last_error
    );
    assert!(h.linear.comments().is_empty());
}

#[tokio::test]
async fn one_workspace_cannot_comment_through_another_workspaces_linear_team() {
    let h = harness().await;
    h.allow_change().await;
    h.allow(h.b.workspace_id, EgressSurface::Linear, TEAM).await;
    h.route(
        &h.a,
        &change_to_linear("changed", &h.base_sha, TEAM, None),
        1,
    )
    .await;
    h.drain().await;

    assert_eq!(h.delivery(&h.a, "linear").await.status, status::SKIPPED);
    assert!(h.linear.comments().is_empty());
}

#[tokio::test]
async fn an_issue_outside_the_named_team_is_refused_before_any_write() {
    // The result names a blessed team, but the issue lives in another one.
    let h = harness_with(RecordingLinear::with_issue(OTHER_TEAM)).await;
    h.allow_change().await;
    h.allow(h.a.workspace_id, EgressSurface::Linear, TEAM).await;
    h.route(
        &h.a,
        &change_to_linear("no_change", &h.base_sha, TEAM, Some("In Review")),
        1,
    )
    .await;
    h.drain().await;

    let row = h.delivery(&h.a, "linear").await;
    assert_eq!(row.status, status::SKIPPED);
    assert!(
        row.last_error
            .as_deref()
            .unwrap()
            .contains("is not in team"),
        "{:?}",
        row.last_error
    );
    assert!(h.linear.comments().is_empty());
    assert!(h.linear.moves().is_empty());
}

#[tokio::test]
async fn a_new_result_edits_the_linear_comment_and_a_deleted_one_is_posted_again() {
    let h = harness().await;
    h.allow_change().await;
    h.allow(h.a.workspace_id, EgressSurface::Linear, TEAM).await;
    h.route(
        &h.a,
        &change_to_linear("no_change", &h.base_sha, TEAM, None),
        1,
    )
    .await;
    h.drain().await;
    assert_eq!(h.linear.comments().len(), 1);

    let mut second = change_to_linear("seat_error", &h.base_sha, TEAM, None);
    second["error_detail"] = json!("the seat timed out");
    h.route(&h.a, &second, 2).await;
    h.drain().await;
    let comments = h.linear.comments();
    assert_eq!(comments.len(), 1, "edited in place: {comments:?}");
    assert_eq!(*h.linear.updates.lock().unwrap(), 1);
    assert!(
        comments[0].2.contains("the seat timed out"),
        "{}",
        comments[0].2
    );

    // Someone deleted the comment in Linear: the next result posts a new one.
    h.linear.comments.lock().unwrap().clear();
    h.route(
        &h.a,
        &change_to_linear("no_change", &h.base_sha, TEAM, None),
        3,
    )
    .await;
    h.drain().await;
    assert_eq!(h.linear.comments().len(), 1);
    assert_eq!(h.delivery(&h.a, "linear").await.status, status::DELIVERED);
}

#[tokio::test]
async fn a_state_the_team_does_not_have_still_lands_the_comment() {
    let h = harness().await;
    h.allow_change().await;
    h.allow(h.a.workspace_id, EgressSurface::Linear, TEAM).await;
    h.route(
        &h.a,
        &change_to_linear("no_change", &h.base_sha, TEAM, Some("Shipped to the moon")),
        1,
    )
    .await;
    h.drain().await;

    assert_eq!(h.linear.comments().len(), 1);
    assert!(h.linear.moves().is_empty());
    assert_eq!(h.delivery(&h.a, "linear").await.status, status::DELIVERED);
}

#[tokio::test]
async fn a_status_maidan_has_never_seen_is_answered_like_seat_error_with_its_reason() {
    let h = harness().await;
    h.allow_change().await;
    h.allow(h.a.workspace_id, EgressSurface::Linear, TEAM).await;
    let mut result = change_to_linear("check_failed", &h.base_sha, TEAM, None);
    result["error_detail"] = json!("cargo test failed: 2 tests");
    h.route(&h.a, &result, 1).await;
    h.drain().await;

    let branch = h.delivery(&h.a, "github_branch").await;
    assert_eq!(branch.status, status::SKIPPED);
    assert!(
        h.fake.lock().unwrap().requests.is_empty(),
        "no GitHub write for a status that is not `changed`"
    );
    let posts = h.slack_posts();
    assert_eq!(posts.len(), 1);
    let comments = h.linear.comments();
    assert_eq!(comments.len(), 1);
    for body in [&posts[0].2, &comments[0].2] {
        assert!(body.contains("`check_failed`"), "{body}");
        assert!(body.contains("nothing was committed"), "{body}");
        assert!(body.contains("cargo test failed: 2 tests"), "{body}");
    }
}

#[tokio::test]
async fn a_change_to_a_pull_request_in_the_merge_queue_is_refused_before_any_write() {
    let mut h = harness().await;
    h.state.merge_queue_labels =
        maidan_types::MergeQueueLabels::parse(&format!("{REPO}=ready-to-merge")).unwrap();
    h.allow_change().await;
    let result = change("changed", &h.base_sha, BRANCH, DIFF);
    h.route(&h.a, &result, 1).await;
    h.drain().await;
    assert_eq!(
        h.delivery(&h.a, "github_branch").await.status,
        status::DELIVERED
    );

    // The pull request is queued; a later delivery to its branch would land
    // apart from the head the queue took.
    let writes_before = {
        let mut f = h.fake.lock().unwrap();
        f.pulls[0].labels.push("ready-to-merge".into());
        f.writes().len()
    };
    h.route(&h.a, &result, 2).await;
    h.drain().await;
    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::SKIPPED);
    let reason = row.last_error.unwrap_or_default();
    assert!(
        reason.contains("merge queue") && reason.contains("ready-to-merge"),
        "{reason}"
    );
    assert_eq!(
        h.fake.lock().unwrap().writes().len(),
        writes_before,
        "nothing written"
    );
}

#[tokio::test]
async fn each_delivered_round_counts_toward_the_swarm_metrics() {
    let h = harness().await;
    h.allow_change().await;
    let result = change("changed", &h.base_sha, BRANCH, DIFF);
    h.route(&h.a, &result, 1).await;
    h.drain().await;
    // A retried delivery of the same commit is not another round.
    h.route(&h.a, &result, 2).await;
    h.drain().await;
    let pulls = h
        .store
        .list_change_pulls(h.a.workspace_id, None, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(pulls.len(), 1);
    assert_eq!(pulls[0].rounds.len(), 1);
    assert_eq!(
        pulls[0].rounds[0].base_behind,
        Some(2),
        "from GitHub's compare"
    );
    assert!(pulls[0].rounds[0].diff_lines > 0);

    // GitHub reports it merged at the delivered head; the metrics read sees it.
    let number = {
        let mut f = h.fake.lock().unwrap();
        f.pulls[0].merged_at = Some("2026-10-14T12:00:00Z".into());
        f.pulls[0].number
    };
    let secret = maidan_auth::TokenSecret::generate();
    h.store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id: h.a.workspace_id,
            member_id: h.a.member_id,
            app_installation_id: None,
            token_hash: maidan_auth::hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![maidan_auth::capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let response = tower::ServiceExt::oneshot(
        maidan_server::router(h.state.clone()),
        axum::http::Request::get(format!("/workspaces/{}/swarm-metrics", h.a.workspace_id.0))
            .header("authorization", format!("Bearer {}", secret.as_str()))
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let metrics: Value = serde_json::from_slice(&bytes).unwrap();
    let task = &metrics["tasks"][0];
    assert_eq!(task["pull_number"], number);
    assert_eq!(task["merged_without_edits"], true, "{metrics}");
    assert_eq!(task["rounds"].as_array().unwrap().len(), 1);
    assert_eq!(metrics["repos"][0]["merged"], 1);

    // The read only recorded the close; the sweep still lands the thread.
    assert_eq!(change_sweep::sweep_once(&h.state).await.landed, 1);
    assert_eq!(
        h.store.get_thread(h.a.thread_id).await.unwrap().state,
        ThreadState::Closed
    );
}

/// A merged pull request closes its task on the next sweep, with
/// `ThreadLanded`, and the task waiting on it becomes claimable. Another
/// workspace's tracked pull request on the same repository, still open, is
/// left alone.
#[tokio::test]
async fn a_merged_change_is_checked_once_for_a_revert_follow_up_fixes_and_human_effort() {
    let h = harness().await;
    h.allow_change().await;
    let mut result = change("changed", &h.base_sha, BRANCH, DIFF);
    result["linear_issue"] = json!("ENG-77");
    h.route(&h.a, &result, 1).await;
    h.drain().await;
    let ago = |hours: i64| {
        (Utc::now() - chrono::Duration::hours(hours))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    let merge_sha = "e".repeat(40);
    let revert_sha = "9".repeat(40);
    let number = {
        let mut f = h.fake.lock().unwrap();
        let head = f.refs[BRANCH].clone();
        let number = f.pulls[0].number;
        f.pulls[0].merged_at = Some(ago(6));
        f.merge_shas.insert(number, merge_sha.clone());
        // The delivered commit, and one a person pushed.
        f.pull_commits.insert(number, vec![head, "d".repeat(40)]);
        f.reviews = vec![
            (
                number,
                "reviews",
                json!({"user": {"login": "coderabbitai[bot]", "type": "Bot"}, "state": "COMMENTED", "submitted_at": ago(5)}),
            ),
            (
                number,
                "reviews",
                json!({"user": {"login": "alice", "type": "User"}, "state": "APPROVED", "submitted_at": ago(4)}),
            ),
            (
                number,
                "comments",
                json!({"user": {"login": "bob", "type": "User"}, "created_at": ago(5)}),
            ),
            (
                number,
                "comments",
                json!({"user": {"login": "helper[bot]", "type": "User"}, "created_at": ago(5)}),
            ),
            (
                number,
                "reviews",
                json!({"user": {"login": "carol", "type": "User"}, "state": "PENDING"}),
            ),
        ];
        f.base_commits = vec![
            ("8".repeat(40), "Unrelated (#40)".into()),
            (
                revert_sha.clone(),
                format!("Revert \"Greet the world\"\n\nThis reverts commit {merge_sha}."),
            ),
        ];
        f.merged_elsewhere = vec![
            json!({"number": 50, "title": "Fix the greeting", "body": format!("Fixes #{number}"), "merged_at": ago(1)}),
            json!({"number": 51, "title": "Docs", "body": format!("Part of #{number}"), "merged_at": ago(1)}),
            json!({"number": 52, "title": "ENG-77: empty names", "body": "", "merged_at": ago(1)}),
            json!({"number": 53, "title": "Older fix", "body": format!("Fixes #{number}"), "merged_at": ago(9)}),
        ];
        number
    };

    change_sweep::sweep_once(&h.state).await;
    let pulls = h
        .store
        .list_change_pulls(h.a.workspace_id, None, None, None, None, None)
        .await
        .unwrap();
    let pull = pulls.iter().find(|p| p.pull_number == number).unwrap();
    assert_eq!(pull.linear_issue.as_deref(), Some("ENG-77"));
    let outcome = &pull.outcome;
    assert_eq!(
        outcome.merge_commit_sha.as_deref(),
        Some(merge_sha.as_str())
    );
    assert!(outcome.reverted_at.is_some());
    assert_eq!(outcome.revert_ref.as_deref(), Some(revert_sha.as_str()));
    assert_eq!(outcome.followup_fixes, vec![50, 52]);
    assert_eq!(
        outcome.human_commits,
        Some(1),
        "the delivered commit is not a person's"
    );
    assert_eq!(
        outcome.human_reviews,
        Some(2),
        "bots and a pending review are not counted"
    );

    let token = read_token(&h, h.a.workspace_id, h.a.member_id).await;
    let (code, metrics) = get_json(
        &h,
        &token,
        &format!("/workspaces/{}/swarm-metrics", h.a.workspace_id.0),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let task = &metrics["tasks"][0];
    assert_eq!(task["reverted"], true);
    assert_eq!(task["followup_fixes"], json!([50, 52]));
    assert_eq!(task["human_commits"], 1);
    assert_eq!(task["linear_issue"], "ENG-77");
    assert_eq!(metrics["repos"][0]["reverted"], 1);
    assert_eq!(metrics["repos"][0]["revert_rate"], 1.0);

    // The next sweeps inside the hour read nothing more for it.
    let outcome_reads = |h: &Harness| {
        h.fake
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| {
                r.starts_with("GET ")
                    && (r.ends_with("/commits")
                        || r.ends_with("/reviews")
                        || r.ends_with("/comments"))
            })
            .count()
    };
    let before = outcome_reads(&h);
    assert_eq!(before, 4, "base commits, pull commits, reviews, comments");
    change_sweep::sweep_once(&h.state).await;
    change_sweep::sweep_once(&h.state).await;
    assert_eq!(outcome_reads(&h), before);
}

#[tokio::test]
async fn the_swarm_metrics_read_only_the_named_linear_issues() {
    let h = harness().await;
    h.allow_change().await;
    let mut result = change("changed", &h.base_sha, BRANCH, DIFF);
    result["linear_issue"] = json!("ENG-77");
    h.route(&h.a, &result, 1).await;
    h.drain().await;
    let token = read_token(&h, h.a.workspace_id, h.a.member_id).await;
    let read = |query: &str| format!("/workspaces/{}/swarm-metrics{query}", h.a.workspace_id.0);
    let (_, pilot) = get_json(&h, &token, &read("?linear_issue=ENG-9,ENG-77")).await;
    assert_eq!(pilot["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(pilot["repos"][0]["tasks"], 1);
    let (_, other) = get_json(&h, &token, &read("?linear_issue=ENG-9")).await;
    assert!(other["tasks"].as_array().unwrap().is_empty());
    assert!(other["repos"].as_array().unwrap().is_empty());
    let (code, _) = get_json(&h, &token, &read("?linear_issue=eng-77")).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_merged_pull_request_closes_its_task_on_the_next_sweep() {
    let h = harness().await;
    h.allow_change().await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;
    let dependent = h
        .store
        .create_thread(NewThread {
            channel_id: h.a.channel_id,
            parent_thread_id: None,
            title: Some("after the change".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id;
    h.store
        .add_thread_dependency(dependent, h.a.thread_id)
        .await
        .unwrap();
    h.store
        .set_thread_result(h.b.thread_id, h.b.member_id, &json!({"summary": "theirs"}))
        .await
        .unwrap();
    h.store
        .record_change_round(maidan_types::NewChangeRound {
            workspace_id: h.b.workspace_id,
            thread_id: h.b.thread_id,
            repo: REPO.into(),
            pull_number: 999,
            commit_sha: "f".repeat(40),
            base_sha: h.base_sha.clone(),
            base_behind: None,
            diff_lines: 1,
            paths: Vec::new(),
            base_branch: None,
            rebase: false,
            previous_head: None,
            linear_issue: None,
        })
        .await
        .unwrap();

    assert_eq!(
        change_sweep::sweep_once(&h.state).await,
        change_sweep::SweepStats::default(),
        "nothing has closed yet"
    );
    let number = {
        let mut f = h.fake.lock().unwrap();
        f.pulls[0].merged_at = Some("2026-10-15T12:00:00Z".into());
        f.pulls[0].number
    };
    assert_eq!(change_sweep::sweep_once(&h.state).await.landed, 1);
    assert_eq!(
        h.store.get_thread(h.a.thread_id).await.unwrap().state,
        ThreadState::Closed
    );
    let events = h
        .store
        .list_events_after(h.a.workspace_id, 0, 500)
        .await
        .unwrap();
    let landed: Vec<_> = events
        .iter()
        .filter(|e| e.kind == EventKind::ThreadLanded)
        .collect();
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].thread_id, Some(h.a.thread_id));
    assert_eq!(landed[0].payload["repo"], REPO);
    assert_eq!(landed[0].payload["pr_number"], number);
    let claimed = h
        .store
        .claim_next_thread(h.a.channel_id, h.a.member_id, None, None)
        .await
        .unwrap();
    assert_eq!(
        claimed.map(|t| t.id),
        Some(dependent),
        "the task waiting on the change is claimable"
    );

    assert_eq!(
        h.store.get_thread(h.b.thread_id).await.unwrap().state,
        ThreadState::Open
    );
    assert!(!h
        .store
        .list_events_after(h.b.workspace_id, 0, 500)
        .await
        .unwrap()
        .iter()
        .any(|e| e.kind == EventKind::ThreadLanded));
    assert_eq!(
        h.store
            .list_open_change_pulls(h.b.workspace_id, 10)
            .await
            .unwrap(),
        vec![(REPO.to_string(), 999)]
    );
    assert_eq!(
        change_sweep::sweep_once(&h.state).await.landed,
        0,
        "a merge lands its thread once"
    );
}

/// A pull request closed without merging leaves its task open, with one
/// notice that a second sweep does not repeat.
#[tokio::test]
async fn a_pull_request_closed_without_merging_leaves_its_task_open_with_one_notice() {
    let h = harness().await;
    h.allow_change().await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;
    let number = {
        let mut f = h.fake.lock().unwrap();
        f.pulls[0].closed_at = Some("2026-10-15T12:00:00Z".into());
        f.pulls[0].number
    };
    assert_eq!(
        change_sweep::sweep_once(&h.state).await,
        change_sweep::SweepStats {
            landed: 0,
            noticed: 1,
            rebase_notices: 0,
            linear_notices: 0,
            handed_off: 0,
            ci_notices: 0,
        }
    );
    assert_eq!(
        change_sweep::sweep_once(&h.state).await,
        change_sweep::SweepStats::default(),
        "the notice is not repeated"
    );
    assert_ne!(
        h.store.get_thread(h.a.thread_id).await.unwrap().state,
        ThreadState::Closed
    );
    let notices: Vec<_> = h
        .store
        .list_messages_after(h.a.thread_id, None, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.body == format!("#{number} closed without merging"))
        .collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].metadata["github"]["closed_unmerged"], true);
}

#[tokio::test]
async fn the_pull_request_body_keeps_a_current_state_section_across_rounds() {
    let h = harness().await;
    h.allow_change().await;
    let with_state = |state: &str| {
        let mut result = change("changed", &h.base_sha, BRANCH, DIFF);
        result["pr_state"] = json!({"title": "ignored", "body": state});
        result
    };
    h.route(&h.a, &with_state("Round one: greets the world."), 1)
        .await;
    h.drain().await;
    let opened = h.fake.lock().unwrap().pulls[0].body.clone();
    assert!(opened.contains("Changes the greeting"), "{opened}");
    assert!(
        opened.contains(
            "<!-- pi:current-state -->\nRound one: greets the world.\n<!-- /pi:current-state -->"
        ),
        "{opened}"
    );

    // A reviewer edits the description; the next round replaces only the
    // section, and never touches the title.
    let title = {
        let mut f = h.fake.lock().unwrap();
        f.pulls[0].body = format!("Reviewer note: ship after QA.\n\n{opened}");
        f.pulls[0].title.clone()
    };
    h.route(&h.a, &with_state("Round two: greets the world twice."), 2)
        .await;
    h.drain().await;
    let f = h.fake.lock().unwrap();
    let body = &f.pulls[0].body;
    assert!(body.starts_with("Reviewer note: ship after QA."), "{body}");
    assert!(
        body.contains("Round two: greets the world twice."),
        "{body}"
    );
    assert!(!body.contains("Round one"), "{body}");
    assert_eq!(body.matches("<!-- pi:current-state -->").count(), 1);
    assert_eq!(f.pulls[0].title, title, "the title is left to people");
}

// ---------------------------------------------------------------- notices

const BRIDGE_DIFF: &str = "diff --git a/src/bridge/x.rs b/src/bridge/x.rs\nindex 1111111..2222222 100644\n--- a/src/bridge/x.rs\n+++ b/src/bridge/x.rs\n@@ -1 +1 @@\n-pub fn bridge() {}\n+pub fn bridge() -> u8 { 1 }\n";

/// A bearer token for `member` in `workspace` that may read it.
async fn read_token(h: &Harness, workspace_id: WorkspaceId, member_id: MemberId) -> String {
    let secret = maidan_auth::TokenSecret::generate();
    h.store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: maidan_auth::hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![maidan_auth::capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn get_json(h: &Harness, token: &str, path: &str) -> (StatusCode, Value) {
    let response = tower::ServiceExt::oneshot(
        maidan_server::router(h.state.clone()),
        axum::http::Request::get(path)
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// An agent holding `glob` in `repo` on a new thread in `t`'s channel: a new
/// member, its thread, the footprint, and the claim.
async fn agent_holding(
    h: &Harness,
    t: &Tenant,
    handle: &str,
    repo: &str,
    glob: &str,
) -> (MemberId, ThreadId) {
    let member = h
        .store
        .create_member(NewMember {
            workspace_id: t.workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id;
    let thread = h
        .store
        .create_thread(NewThread {
            channel_id: t.channel_id,
            parent_thread_id: None,
            title: Some(format!("{handle}'s work")),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id;
    h.store
        .set_thread_footprint(
            thread,
            Some(maidan_types::Footprint {
                repo: repo.into(),
                mode: maidan_types::FootprintMode::Shared,
                items: vec![maidan_types::FootprintItem {
                    kind: maidan_types::FootprintKind::Glob,
                    value: glob.into(),
                }],
            }),
        )
        .await
        .unwrap();
    assert!(h.store.claim_thread(thread, member).await.unwrap().claimed);
    (member, thread)
}

async fn notice_events(h: &Harness, ws: WorkspaceId) -> Vec<maidan_types::StoredEvent> {
    h.store
        .list_events_after(ws, 0, 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == EventKind::ThreadNotice)
        .collect()
}

/// Two agents in one repository: A's pull request touching
/// `src/bridge/x.rs` merges, and B, holding `src/bridge/**`, is told to
/// rebase exactly once: a `ThreadNotice`, one message, and the notice on its
/// thread read (REST and the MCP context pack). C, holding
/// `src/gateway/**`, and an agent holding `src/bridge/**` in another
/// workspace hear nothing, and the next sweep repeats nothing.
#[tokio::test]
async fn a_merge_tells_the_agent_holding_its_paths_to_rebase_once() {
    let h = harness().await;
    h.allow_change().await;
    let (b_member, b_thread) = agent_holding(&h, &h.a, "agent-b", REPO, "src/bridge/**").await;
    let (_, c_thread) = agent_holding(&h, &h.a, "agent-c", REPO, "src/gateway/**").await;
    let (_, foreign_thread) = agent_holding(&h, &h.b, "agent-d", REPO, "src/bridge/**").await;

    h.route(
        &h.a,
        &change("changed", &h.base_sha, BRANCH, BRIDGE_DIFF),
        1,
    )
    .await;
    h.drain().await;
    assert_eq!(
        h.delivery(&h.a, "github_branch").await.status,
        status::DELIVERED
    );
    assert_eq!(
        change_sweep::sweep_once(&h.state).await.rebase_notices,
        0,
        "not merged yet"
    );
    let number = {
        let mut f = h.fake.lock().unwrap();
        f.pulls[0].merged_at = Some("2026-10-15T12:00:00Z".into());
        f.pulls[0].number
    };
    let stats = change_sweep::sweep_once(&h.state).await;
    assert_eq!((stats.landed, stats.rebase_notices), (1, 1));

    let events = notice_events(&h, h.a.workspace_id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].thread_id, Some(b_thread));
    assert_eq!(events[0].payload["notice"], "rebase_needed");
    assert_eq!(events[0].payload["repo"], REPO);
    assert_eq!(events[0].payload["pr_number"], number);
    assert_eq!(events[0].payload["paths"], json!(["src/bridge/x.rs"]));
    assert_eq!(events[0].payload["base_branch"], "dev");
    assert!(notice_events(&h, h.b.workspace_id).await.is_empty());
    let said: Vec<_> = h
        .store
        .list_messages_after(b_thread, None, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.metadata["change_notice"]["kind"] == "rebase_needed")
        .collect();
    assert_eq!(said.len(), 1);
    assert!(
        said[0].body.contains(&format!("{REPO}#{number}")),
        "{}",
        said[0].body
    );
    assert!(said[0].body.contains("src/bridge/x.rs"), "{}", said[0].body);
    assert_eq!(
        said[0].author_id, b_member,
        "the claim holder, no owner set"
    );
    for quiet in [c_thread, foreign_thread] {
        assert!(h
            .store
            .list_thread_notices(quiet, 10)
            .await
            .unwrap()
            .is_empty());
        assert!(h
            .store
            .list_messages_after(quiet, None, 100)
            .await
            .unwrap()
            .is_empty());
    }

    let token = read_token(&h, h.a.workspace_id, h.a.member_id).await;
    let (code, thread) = get_json(&h, &token, &format!("/threads/{}", b_thread.0)).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(thread["notices"].as_array().unwrap().len(), 1, "{thread}");
    assert_eq!(thread["notices"][0]["kind"], "rebase_needed");
    assert_eq!(thread["notices"][0]["paths"], json!(["src/bridge/x.rs"]));
    let (_, quiet) = get_json(&h, &token, &format!("/threads/{}", c_thread.0)).await;
    assert!(quiet.get("notices").is_none(), "{quiet}");
    let pack = maidan_mcp::context::get_thread_context(
        h.store.as_ref(),
        &json!({ "thread_id": b_thread.0 }),
    )
    .await
    .unwrap();
    assert_eq!(pack["notices"][0]["kind"], "rebase_needed", "{pack}");
    assert_eq!(pack["notices"][0]["pull_number"], number);
    // Another tenant's token reads none of it.
    let other = read_token(&h, h.b.workspace_id, h.b.member_id).await;
    let (code, _) = get_json(&h, &other, &format!("/threads/{}", b_thread.0)).await;
    assert_ne!(code, StatusCode::OK);

    assert_eq!(
        change_sweep::sweep_once(&h.state).await,
        change_sweep::SweepStats::default(),
        "a merge notifies once"
    );
    assert_eq!(notice_events(&h, h.a.workspace_id).await.len(), 1);
    assert_eq!(
        h.store
            .list_thread_notices(b_thread, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// A round delivered on a base `MAIDAN_STALE_BASE_COMMITS` (50 by default)
/// or more behind gets one `stale_base` notice on its thread; a retried
/// delivery of the same commit adds none.
#[tokio::test]
async fn a_stale_base_delivery_gets_a_notice() {
    let h = harness().await;
    h.allow_change().await;
    h.fake.lock().unwrap().behind = Some(60);
    let result = change("changed", &h.base_sha, BRANCH, DIFF);
    h.route(&h.a, &result, 1).await;
    h.drain().await;
    h.route(&h.a, &result, 2).await;
    h.drain().await;
    let notices = h
        .store
        .list_thread_notices(h.a.thread_id, 10)
        .await
        .unwrap();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].kind, maidan_types::ThreadNoticeKind::StaleBase);
    assert_eq!(notices[0].base_behind, Some(60));
    assert_eq!(notices[0].base_branch.as_deref(), Some("dev"));
    let events = notice_events(&h, h.a.workspace_id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["notice"], "stale_base");
    let said: Vec<_> = h
        .store
        .list_messages_after(h.a.thread_id, None, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.metadata["change_notice"]["kind"] == "stale_base")
        .collect();
    assert_eq!(said.len(), 1);
    assert!(
        said[0].body.contains("60 commits behind dev"),
        "{}",
        said[0].body
    );
}

/// A round on a fresh base (2 behind) gets no notice.
#[tokio::test]
async fn a_fresh_base_delivery_gets_no_notice() {
    let h = harness().await;
    h.allow_change().await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;
    assert_eq!(
        h.delivery(&h.a, "github_branch").await.status,
        status::DELIVERED
    );
    assert!(h
        .store
        .list_thread_notices(h.a.thread_id, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(notice_events(&h, h.a.workspace_id).await.is_empty());
}

// ---------------------------------------------------------------- squash-rebase rounds

/// A squash-rebase result: `base_sha` is the base branch's head and `diff`
/// the whole branch against it.
fn rebase_change(base_sha: &str, diff: &str) -> Value {
    let mut result = change("changed", base_sha, BRANCH, diff);
    result["rebase"] = json!(true);
    result
}

/// Tenant A delivers a normal first round, then `dev` moves on under it.
/// Returns the delivered head and the new base.
async fn delivered_then_base_moved(h: &Harness) -> (String, String) {
    h.allow_change().await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;
    assert_eq!(
        h.delivery(&h.a, "github_branch").await.status,
        status::DELIVERED
    );
    let mut f = h.fake.lock().unwrap();
    let delivered = f.refs[BRANCH].clone();
    let new_base = f.commit_files(
        &[
            ("README.md", "readme, revised on dev\n"),
            ("src/greeting.txt", GREETING),
            ("src/bridge/x.rs", BRIDGE),
        ],
        vec![h.base_sha.clone()],
    );
    f.refs.insert("dev".into(), new_base.clone());
    (delivered, new_base)
}

#[tokio::test]
async fn a_rebase_round_moves_the_branch_to_one_commit_on_the_new_base_and_a_retry_reuses_it() {
    let h = harness().await;
    let (delivered, new_base) = delivered_then_base_moved(&h).await;
    h.route(&h.a, &rebase_change(&new_base, DIFF), 2).await;
    h.drain().await;

    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::DELIVERED, "{:?}", row.last_error);
    let (head, pull) = {
        let f = h.fake.lock().unwrap();
        let head = f.refs[BRANCH].clone();
        assert_ne!(head, delivered);
        assert_eq!(
            f.commits[&head].parents,
            vec![new_base.clone()],
            "one commit whose only parent is the new base"
        );
        assert_eq!(
            f.file(&head, "src/greeting.txt").as_deref(),
            Some("line one\nhello, world\nline three\n")
        );
        assert_eq!(
            f.file(&head, "README.md").as_deref(),
            Some("readme, revised on dev\n"),
            "the base's own work is kept"
        );
        assert!(f.commits[&head].message.contains("Maidan-Change: "));
        assert_eq!(
            f.forced,
            vec![(BRANCH.to_string(), delivered.clone(), head.clone())]
        );
        assert_eq!(f.pulls.len(), 1, "the same pull request stays open");
        let pull = f.pulls[0].clone();
        assert_eq!((pull.head.as_str(), pull.base.as_str()), (BRANCH, "dev"));
        assert!(pull.merged_at.is_none() && pull.closed_at.is_none());
        (head, pull)
    };
    assert_eq!(
        row.external_ref.as_deref(),
        Some(format!("{head}#{}", pull.number).as_str())
    );

    // Reviewers get the range the force rewrote, on the pull request and in
    // the Slack reply.
    let range = format!("Rebased {delivered}..{head} onto {new_base}");
    assert_eq!(
        h.fake.lock().unwrap().pull_comments,
        vec![(pull.number, range.clone())]
    );
    let reply = h.slack_reply();
    assert!(reply.contains(&range), "{range:?} not in {reply}");
    assert!(reply.contains(&format!("Committed {head}")), "{reply}");

    // The round is recorded as a rebase, on a current base.
    let pulls = h
        .store
        .list_change_pulls(h.a.workspace_id, None, None, None, None, None)
        .await
        .unwrap();
    let rounds: Vec<_> = pulls[0]
        .rounds
        .iter()
        .map(|r| {
            (
                r.commit_sha.clone(),
                r.rebase,
                r.base_behind,
                r.previous_head.clone(),
            )
        })
        .collect();
    assert_eq!(
        rounds,
        vec![
            (delivered.clone(), false, Some(2), None),
            (head.clone(), true, Some(0), Some(delivered.clone())),
        ]
    );
    let metrics =
        maidan_types::swarm_metrics(pulls, maidan_types::FlagLimits::default(), Utc::now());
    assert_eq!(metrics.tasks[0].rebases, 1);

    // A retried delivery finds its own rebase commit and writes nothing,
    // even though the delivered head it was checked against has moved.
    let writes_before = h.fake.lock().unwrap().writes().len();
    let replayed = maidan_store::replay_result_delivery(
        h.store.as_ref(),
        h.a.workspace_id,
        h.a.thread_id,
        row.id,
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
        let f = h.fake.lock().unwrap();
        assert_eq!(f.refs[BRANCH], head);
        assert_eq!(f.forced.len(), 1, "no second force");
        assert_eq!(f.writes().len(), writes_before, "{:?}", f.writes());
    }
    assert_eq!(
        h.delivery(&h.a, "github_branch").await.status,
        status::DELIVERED
    );
}

/// The range comment is best effort: GitHub refusing it leaves the move,
/// the delivery and the recorded range as they are, and Slack still says it.
#[tokio::test]
async fn a_rebase_whose_range_comment_fails_still_lands_and_says_it_in_slack() {
    let h = harness().await;
    let (delivered, new_base) = delivered_then_base_moved(&h).await;
    h.fake.lock().unwrap().fail_comments = true;
    h.route(&h.a, &rebase_change(&new_base, DIFF), 2).await;
    h.drain().await;

    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::DELIVERED, "{:?}", row.last_error);
    let head = {
        let f = h.fake.lock().unwrap();
        assert!(f.pull_comments.is_empty());
        assert_eq!(f.forced.len(), 1);
        f.refs[BRANCH].clone()
    };
    let range = format!("Rebased {delivered}..{head} onto {new_base}");
    let reply = h.slack_reply();
    assert!(reply.contains(&range), "{range:?} not in {reply}");
}

/// The branch reads, refusal, and that nothing was written since `before`.
async fn assert_rebase_refused(h: &Harness, writes_before: usize, head: &str, why: &[&str]) {
    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::SKIPPED);
    let reason = row.last_error.unwrap_or_default();
    for part in why {
        assert!(reason.contains(part), "{part:?} not in {reason}");
    }
    let f = h.fake.lock().unwrap();
    assert_eq!(f.writes().len(), writes_before, "{:?}", f.writes());
    assert!(f.forced.is_empty(), "{:?}", f.forced);
    assert_eq!(f.refs[BRANCH], head);
}

#[tokio::test]
async fn a_rebase_over_a_commit_someone_else_pushed_is_refused_naming_both_shas() {
    let h = harness().await;
    let (delivered, new_base) = delivered_then_base_moved(&h).await;
    let (foreign, writes_before) = {
        let mut f = h.fake.lock().unwrap();
        let foreign = f.commit_files(
            &[("src/greeting.txt", "a person's fix\n")],
            vec![delivered.clone()],
        );
        f.refs.insert(BRANCH.into(), foreign.clone());
        (foreign, f.writes().len())
    };
    h.route(&h.a, &rebase_change(&new_base, DIFF), 2).await;
    h.drain().await;
    assert_rebase_refused(
        &h,
        writes_before,
        &foreign,
        &[&foreign, &delivered, "someone else pushed"],
    )
    .await;
}

#[tokio::test]
async fn a_rebase_whose_base_sha_is_not_the_base_head_is_refused() {
    let h = harness().await;
    let (delivered, new_base) = delivered_then_base_moved(&h).await;
    let writes_before = h.fake.lock().unwrap().writes().len();
    // Built on the old base: a rebase must never move a branch backwards.
    h.route(&h.a, &rebase_change(&h.base_sha, DIFF), 2).await;
    h.drain().await;
    assert_rebase_refused(
        &h,
        writes_before,
        &delivered,
        &[&h.base_sha, &new_base, "not the head of `dev`"],
    )
    .await;
}

#[tokio::test]
async fn a_rebase_of_a_pull_request_in_the_merge_queue_is_refused() {
    let mut h = harness().await;
    h.state.merge_queue_labels =
        maidan_types::MergeQueueLabels::parse(&format!("{REPO}=ready-to-merge")).unwrap();
    let (delivered, new_base) = delivered_then_base_moved(&h).await;
    let writes_before = {
        let mut f = h.fake.lock().unwrap();
        f.pulls[0].labels.push("ready-to-merge".into());
        f.writes().len()
    };
    h.route(&h.a, &rebase_change(&new_base, DIFF), 2).await;
    h.drain().await;
    assert_rebase_refused(&h, writes_before, &delivered, &["merge queue"]).await;
}

#[tokio::test]
async fn a_rebase_with_no_open_pull_request_or_a_diff_that_does_not_apply_writes_nothing() {
    // No pull request: nothing was delivered, so nothing vouches for a force.
    let h = harness().await;
    h.allow_change().await;
    let lone = {
        let mut f = h.fake.lock().unwrap();
        let lone = f.commit_files(&[("src/greeting.txt", "x\n")], vec![h.base_sha.clone()]);
        f.refs.insert(BRANCH.into(), lone.clone());
        lone
    };
    h.route(&h.a, &rebase_change(&h.base_sha, DIFF), 1).await;
    h.drain().await;
    assert_rebase_refused(&h, 0, &lone, &["needs the open pull request"]).await;

    // A whole-branch diff that does not apply to the new base.
    let h = harness().await;
    let (delivered, new_base) = delivered_then_base_moved(&h).await;
    let writes_before = h.fake.lock().unwrap().writes().len();
    let stale = DIFF.replace("-hello\n", "-goodbye\n");
    h.route(&h.a, &rebase_change(&new_base, &stale), 2).await;
    h.drain().await;
    assert_rebase_refused(&h, writes_before, &delivered, &["does not apply"]).await;
}

/// Two workspaces on one pull request: a commit another workspace delivered
/// is someone else's push to this one, so it never vouches for the first
/// workspace's rebase. The workspace that did deliver the head is vouched for
/// by its own row.
#[tokio::test]
async fn another_workspaces_delivery_never_vouches_for_a_rebase() {
    let h = harness().await;
    let (delivered, new_base) = delivered_then_base_moved(&h).await;
    let number = h.fake.lock().unwrap().pulls[0].number;
    // Workspace B's round lands on the same branch and pull request.
    let theirs = {
        let mut f = h.fake.lock().unwrap();
        let theirs = f.commit_files(&[("src/greeting.txt", "theirs\n")], vec![delivered.clone()]);
        f.refs.insert(BRANCH.into(), theirs.clone());
        theirs
    };
    h.store
        .record_change_round(maidan_types::NewChangeRound {
            workspace_id: h.b.workspace_id,
            thread_id: h.b.thread_id,
            repo: REPO.into(),
            pull_number: number,
            commit_sha: theirs.clone(),
            base_sha: delivered.clone(),
            base_behind: None,
            diff_lines: 1,
            paths: Vec::new(),
            base_branch: Some("dev".into()),
            rebase: false,
            previous_head: None,
            linear_issue: None,
        })
        .await
        .unwrap();
    let writes_before = h.fake.lock().unwrap().writes().len();
    h.route(&h.a, &rebase_change(&new_base, DIFF), 2).await;
    h.drain().await;
    assert_rebase_refused(&h, writes_before, &theirs, &[&theirs, &delivered]).await;

    // B, blessed for the same repository, tries the rebase itself.
    h.allow(
        h.b.workspace_id,
        EgressSurface::GithubBranch,
        &format!("{REPO}@dev"),
    )
    .await;
    h.route(&h.b, &rebase_change(&new_base, DIFF), 3).await;
    h.drain().await;
    let row = h.delivery(&h.b, "github_branch").await;
    assert_eq!(row.status, status::DELIVERED, "{:?}", row.last_error);
    let f = h.fake.lock().unwrap();
    assert_eq!(f.forced.len(), 1, "{:?}", f.forced);
    assert_ne!(f.refs[BRANCH], theirs, "B rebased the head it delivered");
}

#[tokio::test]
async fn a_rebase_conflict_is_skipped_with_no_write_and_the_reply_names_the_paths() {
    let h = harness().await;
    h.allow_change().await;
    let mut result = change("rebase_conflict", &h.base_sha, BRANCH, "");
    result["rebase"] = json!(true);
    result["error_detail"] = json!("conflicts in src/greeting.txt and src/bridge/x.rs");
    h.route(&h.a, &result, 1).await;
    h.drain().await;

    let row = h.delivery(&h.a, "github_branch").await;
    assert_eq!(row.status, status::SKIPPED);
    let reason = row.last_error.unwrap_or_default();
    assert!(
        reason.contains("rebase_conflict") && reason.contains("conflicts"),
        "{reason}"
    );
    assert!(
        h.fake.lock().unwrap().requests.is_empty(),
        "no GitHub call at all"
    );
    let posts = h.slack_posts();
    assert_eq!(posts.len(), 1);
    let text = &posts[0].2;
    assert_eq!(posts[0].1.as_deref(), Some(THREAD_TS));
    for part in [
        "rebase_conflict",
        "nothing was committed",
        "src/greeting.txt and src/bridge/x.rs",
    ] {
        assert!(text.contains(part), "{part:?} not in {text}");
    }
}

// ---------------------------------------------------------------- CI watch

/// A token for `member` in `workspace` that may read and write it.
async fn write_token(h: &Harness, workspace_id: WorkspaceId, member_id: MemberId) -> String {
    let secret = maidan_auth::TokenSecret::generate();
    h.store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: maidan_auth::hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                maidan_auth::capability::WORKSPACE_READ.into(),
                maidan_auth::capability::WORKSPACE_WRITE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn post_json(h: &Harness, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = tower::ServiceExt::oneshot(
        maidan_server::router(h.state.clone()),
        axum::http::Request::post(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A check run as GitHub's `statusCheckRollup` lists it; `job` makes it a
/// GitHub Actions job whose log can be read.
fn check_run(name: &str, conclusion: Option<&str>, job: Option<i64>) -> Value {
    json!({
        "__typename": "CheckRun",
        "name": name,
        "status": if conclusion.is_some() { "COMPLETED" } else { "IN_PROGRESS" },
        "conclusion": conclusion,
        "databaseId": job.unwrap_or(9_000),
        "checkSuite": {"app": {"slug": if job.is_some() { "github-actions" } else { "other-ci" }}},
    })
}

/// An open pull request someone else's tool opened, on a head of its own.
fn open_someone_elses_pull(h: &Harness, number: i64, head: &str) -> String {
    let mut f = h.fake.lock().unwrap();
    let base = f.refs["dev"].clone();
    let sha = f.commit_files(&[("README.md", head)], vec![base]);
    f.refs.insert(head.into(), sha.clone());
    f.pulls.push(FakePull {
        number,
        head: head.into(),
        base: "dev".into(),
        title: "Someone else's change".into(),
        body: String::new(),
        draft: false,
        labels: Vec::new(),
        merged_at: None,
        closed_at: None,
    });
    sha
}

async fn thread_view(h: &Harness, token: &str, thread: uuid::Uuid) -> Value {
    get_json(h, token, &format!("/threads/{thread}")).await.1
}

fn log_reads(h: &Harness, job: i64) -> usize {
    let wanted = format!("GET /repos/{REPO}/actions/jobs/{job}/logs");
    h.fake
        .lock()
        .unwrap()
        .requests
        .iter()
        .filter(|r| **r == wanted)
        .count()
}

/// A pull request nobody claimed, watched by repository and number: Maidan
/// makes it a thread, and each sweep records its checks there and says, once
/// per head, that it opened, that a check failed (with the end of the
/// failing job's log, read once) and that the checks are done.
#[tokio::test]
async fn a_watched_pull_says_it_opened_failed_and_finished_once_per_head() {
    let h = harness().await;
    h.allow(h.a.workspace_id, EgressSurface::Github, REPO).await;
    open_someone_elses_pull(&h, 90, "feature/someone");
    h.fake
        .lock()
        .unwrap()
        .checks
        .insert(90, vec![check_run("lint", None, Some(501))]);
    let token = write_token(&h, h.a.workspace_id, h.a.member_id).await;
    let watch = format!("/workspaces/{}/pull-watches", h.a.workspace_id.0);

    let (status, watched) = post_json(
        &h,
        &token,
        &watch,
        json!({"repo": "https://github.com/Example/App", "pull_number": 90}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{watched}");
    assert_eq!(watched["created"], true);
    assert_eq!(watched["repo"], REPO);
    let thread: uuid::Uuid = watched["thread_id"].as_str().unwrap().parse().unwrap();
    let (_, again) = post_json(&h, &token, &watch, json!({"repo": REPO, "pull_number": 90})).await;
    assert_eq!(again["created"], false);
    assert_eq!(again["thread_id"], watched["thread_id"]);
    let (status, _) = post_json(
        &h,
        &token,
        &watch,
        json!({"repo": "other/repo", "pull_number": 90}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "only a blessed repository");

    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 1);
    let view = thread_view(&h, &token, thread).await;
    assert_eq!(view["ci"][0]["status"], "pending", "{view}");
    assert_eq!(view["notices"][0]["kind"], "pr_opened", "{view}");
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 0);

    let log = format!(
        "{}\nerror: assertion failed in lint\n",
        "noise line\n".repeat(600)
    );
    {
        let mut f = h.fake.lock().unwrap();
        f.job_logs.insert(501, log);
        f.checks.insert(
            90,
            vec![
                check_run("lint", Some("FAILURE"), Some(501)),
                check_run("unit tests", None, Some(502)),
            ],
        );
    }
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 1);
    let view = thread_view(&h, &token, thread).await;
    let ci = &view["ci"][0];
    assert_eq!(ci["status"], "failure", "{view}");
    assert_eq!(ci["failing_jobs"], json!(["lint"]));
    let tail = ci["log_tail"].as_str().unwrap();
    assert!(tail.len() <= maidan_types::LOG_TAIL_BYTES, "{}", tail.len());
    assert!(
        tail.ends_with("error: assertion failed in lint\n"),
        "{tail}"
    );
    assert!(tail.starts_with("noise line"), "starts at a line: {tail:?}");
    assert_eq!(view["notices"][0]["kind"], "ci_failed");
    assert_eq!(log_reads(&h, 501), 1);

    assert_eq!(
        change_sweep::sweep_once(&h.state).await.ci_notices,
        0,
        "unit tests still run: nothing new"
    );
    assert_eq!(
        log_reads(&h, 501),
        1,
        "the log is read once per failing head"
    );
    assert!(thread_view(&h, &token, thread).await["ci"][0]["log_tail"].is_string());

    h.fake.lock().unwrap().checks.insert(
        90,
        vec![
            check_run("lint", Some("FAILURE"), Some(501)),
            check_run("unit tests", Some("SUCCESS"), Some(502)),
        ],
    );
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 1);
    let view = thread_view(&h, &token, thread).await;
    assert_eq!(view["notices"][0]["kind"], "checks_done");
    assert_eq!(view["notices"][0]["status"], "failure");
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 0);

    // A push: the new head's checks are news; the open is not said again.
    let new_head = {
        let mut f = h.fake.lock().unwrap();
        let parent = f.refs["feature/someone"].clone();
        let sha = f.commit_files(&[("README.md", "fixed")], vec![parent]);
        f.refs.insert("feature/someone".into(), sha.clone());
        f.checks.insert(
            90,
            vec![
                check_run("lint", Some("SUCCESS"), Some(503)),
                check_run("unit tests", Some("SUCCESS"), Some(504)),
                json!({"__typename": "StatusContext", "context": "ci/legacy", "state": "SUCCESS"}),
            ],
        );
        sha
    };
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 1);
    let view = thread_view(&h, &token, thread).await;
    assert_eq!(view["ci"][0]["head_sha"], new_head);
    assert_eq!(view["ci"][0]["status"], "success");
    assert!(view["ci"][0].get("log_tail").is_none(), "{view}");
    assert_eq!(view["notices"][0]["kind"], "checks_done");
    assert_eq!(view["notices"][0]["status"], "success");
    assert_eq!(view["notices"][0]["head_sha"], new_head);

    let said: Vec<String> = h
        .store
        .list_messages(ThreadId(thread), 50)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect();
    assert_eq!(said.len(), 4, "one message per notice: {said:?}");
    assert!(
        said.iter().any(
            |body| body.starts_with("CI failed on example/app#90") && body.ends_with(": lint.")
        ),
        "{said:?}"
    );
}

/// A pull request the change flow opened is watched without anyone asking:
/// its task's thread hears that it failed.
#[tokio::test]
async fn a_change_flow_pull_is_watched_without_asking() {
    let h = harness().await;
    h.allow_change().await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;
    let number = h.fake.lock().unwrap().pulls[0].number;
    h.fake.lock().unwrap().checks.insert(
        number,
        vec![check_run("unit tests", Some("TIMED_OUT"), Some(601))],
    );
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 3);
    let mut kinds: Vec<&str> = h
        .store
        .list_thread_notices(h.a.thread_id, 10)
        .await
        .unwrap()
        .iter()
        .map(|n| n.kind.as_str())
        .collect();
    kinds.sort_unstable();
    assert_eq!(kinds, vec!["checks_done", "ci_failed", "pr_opened"]);
    let ci = h.store.list_thread_pull_ci(h.a.thread_id).await.unwrap();
    assert_eq!(ci[0].failing_jobs, vec!["unit tests".to_string()]);
    assert_eq!(ci[0].log_tail, None, "a job with no log leaves no tail");
}

/// Tenant B watching a pull request tenant A watches gets its own thread,
/// and still cannot read A's watch thread or watch into A's workspace or
/// channel, even with the repository blessed for B as well.
#[tokio::test]
async fn another_tenant_watches_on_its_own_thread_and_cannot_read_a_watched_pull() {
    let h = harness().await;
    h.allow(h.a.workspace_id, EgressSurface::Github, REPO).await;
    h.allow(h.b.workspace_id, EgressSurface::Github, REPO).await;
    open_someone_elses_pull(&h, 91, "feature/shared");
    let a_token = write_token(&h, h.a.workspace_id, h.a.member_id).await;
    let b_token = write_token(&h, h.b.workspace_id, h.b.member_id).await;
    let (status, watched) = post_json(
        &h,
        &a_token,
        &format!("/workspaces/{}/pull-watches", h.a.workspace_id.0),
        json!({"repo": REPO, "pull_number": 91}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let thread = watched["thread_id"].as_str().unwrap();
    h.fake
        .lock()
        .unwrap()
        .checks
        .insert(91, vec![check_run("lint", Some("FAILURE"), None)]);
    change_sweep::sweep_once(&h.state).await;

    let b_watch = format!("/workspaces/{}/pull-watches", h.b.workspace_id.0);
    let (status, b_watched) = post_json(
        &h,
        &b_token,
        &b_watch,
        json!({"repo": REPO, "pull_number": 91}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "B tracks its own row");
    assert_ne!(b_watched["thread_id"].as_str(), Some(thread), "{b_watched}");
    let (status, _) = post_json(
        &h,
        &b_token,
        &format!("/workspaces/{}/pull-watches", h.a.workspace_id.0),
        json!({"repo": REPO, "pull_number": 92}),
    )
    .await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "not B's workspace: {status}"
    );
    let (status, _) = post_json(
        &h,
        &b_token,
        &b_watch,
        json!({"repo": REPO, "pull_number": 92, "channel_id": h.a.channel_id.0}),
    )
    .await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "not B's channel: {status}"
    );
    let (status, body) = get_json(&h, &b_token, &format!("/threads/{thread}")).await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "{status} {body}"
    );
    assert!(body.get("ci").is_none(), "{body}");
    assert!(
        h.store
            .list_channels(h.a.workspace_id)
            .await
            .unwrap()
            .iter()
            .filter(|c| c.name == maidan_types::DEFAULT_WATCH_CHANNEL)
            .count()
            == 1,
        "B's watch made nothing in A"
    );
}

/// A watched pull request's merge-queue state is recorded each tick:
/// labelled ready, queued, dequeued with GitHub's reason, and landed once it
/// merges. Entering and leaving the queue are each said once.
#[tokio::test]
async fn a_watched_pull_says_when_it_enters_and_leaves_the_merge_queue() {
    let h = harness().await;
    h.allow(h.a.workspace_id, EgressSurface::Github, REPO).await;
    open_someone_elses_pull(&h, 94, "feature/queued");
    {
        let mut f = h.fake.lock().unwrap();
        f.checks
            .insert(94, vec![check_run("lint", Some("SUCCESS"), Some(801))]);
        f.pulls.last_mut().unwrap().labels = vec!["ready-to-merge".into()];
    }
    let token = write_token(&h, h.a.workspace_id, h.a.member_id).await;
    let (_, watched) = post_json(
        &h,
        &token,
        &format!("/workspaces/{}/pull-watches", h.a.workspace_id.0),
        json!({"repo": REPO, "pull_number": 94}),
    )
    .await;
    let thread: uuid::Uuid = watched["thread_id"].as_str().unwrap().parse().unwrap();
    let newest = |view: &Value| view["notices"][0].clone();

    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 2);
    let view = thread_view(&h, &token, thread).await;
    assert_eq!(
        view["ci"][0]["merge_queue"]["state"], "ready_to_merge",
        "{view}"
    );

    h.fake.lock().unwrap().queued.push(94);
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 1);
    let view = thread_view(&h, &token, thread).await;
    assert_eq!(view["ci"][0]["merge_queue"]["state"], "queued");
    assert_eq!(newest(&view)["kind"], "merge_queued");
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 0);

    {
        let mut f = h.fake.lock().unwrap();
        f.queued.clear();
        f.dequeued.insert(94, "MERGE_CONFLICT".into());
    }
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 1);
    let view = thread_view(&h, &token, thread).await;
    assert_eq!(
        view["ci"][0]["merge_queue"],
        json!({"state": "dequeued", "reason": "MERGE_CONFLICT"})
    );
    assert_eq!(newest(&view)["kind"], "merge_dequeued");
    assert_eq!(newest(&view)["reasons"], json!(["MERGE_CONFLICT"]));
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 0);

    h.fake.lock().unwrap().pulls.last_mut().unwrap().merged_at =
        Some("2026-10-21T12:00:00Z".into());
    assert_eq!(change_sweep::sweep_once(&h.state).await.landed, 1);
    let view = thread_view(&h, &token, thread).await;
    assert_eq!(view["ci"][0]["merge_queue"]["state"], "landed", "{view}");
    assert_eq!(view["state"], "closed");
}

/// A stack tool that lands a pull request's head itself and then closes the
/// pull request lands its task, through the same `ThreadLanded` as a merge.
#[tokio::test]
async fn a_pull_closed_after_its_head_landed_on_the_base_lands_its_task() {
    let h = harness().await;
    h.allow_change().await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;
    let number = h.fake.lock().unwrap().pulls[0].number;
    h.fake
        .lock()
        .unwrap()
        .checks
        .insert(number, vec![check_run("lint", Some("SUCCESS"), Some(901))]);
    change_sweep::sweep_once(&h.state).await;
    // The tool lands the head; a tick sees the base caught up before the
    // close, and the head still counts as having been ahead.
    {
        let mut f = h.fake.lock().unwrap();
        let head = f.refs[BRANCH].clone();
        f.on_base.push(head);
    }
    change_sweep::sweep_once(&h.state).await;
    h.fake.lock().unwrap().pulls[0].closed_at = Some("2026-10-21T12:00:00Z".into());
    let stats = change_sweep::sweep_once(&h.state).await;
    assert_eq!((stats.landed, stats.noticed), (1, 0), "{stats:?}");
    let thread = h.store.get_thread(h.a.thread_id).await.unwrap();
    assert_eq!(thread.state, ThreadState::Closed);
    let landed = h
        .store
        .list_events_after(h.a.workspace_id, 0, 500)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == EventKind::ThreadLanded && e.thread_id == Some(h.a.thread_id))
        .count();
    assert_eq!(landed, 1);
}

/// A branch reset to its base and then closed has its head on the base too,
/// but it was never ahead of it: that close lands nothing and is said as a
/// close without a merge.
#[tokio::test]
async fn a_branch_reset_to_its_base_and_closed_does_not_land() {
    let h = harness().await;
    h.allow_change().await;
    h.route(&h.a, &change("changed", &h.base_sha, BRANCH, DIFF), 1)
        .await;
    h.drain().await;
    let number = {
        let mut f = h.fake.lock().unwrap();
        let base = h.base_sha.clone();
        f.refs.insert(BRANCH.into(), base.clone());
        f.on_base.push(base);
        f.pulls[0].number
    };
    h.fake
        .lock()
        .unwrap()
        .checks
        .insert(number, vec![check_run("lint", Some("SUCCESS"), Some(902))]);
    change_sweep::sweep_once(&h.state).await;
    h.fake.lock().unwrap().pulls[0].closed_at = Some("2026-10-21T12:00:00Z".into());
    let stats = change_sweep::sweep_once(&h.state).await;
    assert_eq!((stats.landed, stats.noticed), (0, 1), "{stats:?}");
    assert_ne!(
        h.store.get_thread(h.a.thread_id).await.unwrap().state,
        ThreadState::Closed
    );
}

/// The same read says what else holds a watched pull request back: an
/// unresolved review thread (a review bot's counts), a merge state such as
/// `BLOCKED` or `BEHIND`, a required check that never reported. Each change
/// is said once, as `gate_blocked` with the reasons or `gate_clear`.
#[tokio::test]
async fn a_watched_pull_says_what_holds_it_back_when_that_changes() {
    let h = harness().await;
    h.allow(h.a.workspace_id, EgressSurface::Github, REPO).await;
    open_someone_elses_pull(&h, 93, "feature/held");
    {
        let mut f = h.fake.lock().unwrap();
        f.checks
            .insert(93, vec![check_run("lint", Some("SUCCESS"), Some(701))]);
        f.required_checks = Some(vec!["lint".into(), "e2e".into()]);
        f.review_threads.insert(
            93,
            vec![(false, "review-bot[bot]".into()), (true, "someone".into())],
        );
        f.merge_states.insert(93, "BLOCKED".into());
    }
    let token = write_token(&h, h.a.workspace_id, h.a.member_id).await;
    let (_, watched) = post_json(
        &h,
        &token,
        &format!("/workspaces/{}/pull-watches", h.a.workspace_id.0),
        json!({"repo": REPO, "pull_number": 93}),
    )
    .await;
    let thread: uuid::Uuid = watched["thread_id"].as_str().unwrap().parse().unwrap();
    let gate_notices = |view: &Value| -> Vec<Value> {
        view["notices"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["kind"].as_str().unwrap().starts_with("gate_"))
            .cloned()
            .collect()
    };

    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 3);
    let view = thread_view(&h, &token, thread).await;
    let ci = &view["ci"][0];
    assert_eq!(ci["merge_state"], "BLOCKED", "{view}");
    assert_eq!(ci["unresolved_threads"], 1);
    assert_eq!(
        ci["unresolved_thread_links"][0]["author"],
        "review-bot[bot]"
    );
    assert_eq!(ci["missing_checks"], json!(["e2e"]));
    let held = json!([
        "1 unresolved review thread",
        "merge state BLOCKED",
        "required check `e2e` never reported"
    ]);
    assert_eq!(ci["gate_reasons"], held);
    let gates = gate_notices(&view);
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0]["kind"], "gate_blocked");
    assert_eq!(gates[0]["reasons"], held);
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 0);

    {
        let mut f = h.fake.lock().unwrap();
        f.review_threads
            .insert(93, vec![(true, "review-bot[bot]".into())]);
        f.merge_states.insert(93, "CLEAN".into());
        f.required_checks = Some(vec!["lint".into()]);
    }
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 1);
    let view = thread_view(&h, &token, thread).await;
    assert_eq!(gate_notices(&view)[0]["kind"], "gate_clear", "{view}");
    assert!(view["ci"][0].get("gate_reasons").is_none());
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 0);

    h.fake
        .lock()
        .unwrap()
        .merge_states
        .insert(93, "BEHIND".into());
    assert_eq!(change_sweep::sweep_once(&h.state).await.ci_notices, 1);
    let view = thread_view(&h, &token, thread).await;
    assert_eq!(
        gate_notices(&view)[0]["reasons"],
        json!(["merge state BEHIND"])
    );
}
