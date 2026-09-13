//! Cluster 380.2: the egress worker posts inline GitHub review comments.
//!
//! After a successful Cluster 379 summary comment, a `reviewed` envelope with
//! `head_sha` and usable findings becomes `POST /repos/{repo}/pulls/{n}/reviews`
//! with `commit_id = head_sha` (never the live PR head), `event: COMMENT`,
//! `side: RIGHT`, and `line`/`start_line` from the 380.1 post-image mapping.
//! Missing sha, unusable findings, a non-`reviewed` status, Slack, and GitHub
//! 404/422 skip the review without sinking the summary.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    egress_body, egress_worker,
    github::{GithubError, GithubIssueComment, GithubSender},
    notification_router,
    slack::{SlackError, SlackSender},
    AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    status, EgressSurface, Event, ExternalRef, GithubDiffSide, GithubReviewComment, MemberKind,
    NewChannel, NewEgressTarget, NewMember, NewThread, NewWorkspace, ThreadId,
    WAITER_RESULT_SCHEMA,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const FIXTURE: &str = include_str!("../../maidan-types/tests/fixtures/pi_waiter_result_v1.json");
const HEAD_SHA: &str = "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911";

struct ReviewCall {
    repo: String,
    pull: i64,
    commit_id: String,
    comments: Vec<GithubReviewComment>,
}

struct RecordingGithub {
    next_id: AtomicI64,
    posts: Mutex<Vec<(String, i64, String)>>,
    updates: Mutex<Vec<(i64, String)>>,
    reviews: Mutex<Vec<ReviewCall>>,
    fail_review: Mutex<Option<GithubError>>,
}

impl RecordingGithub {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            next_id: AtomicI64::new(1),
            posts: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
            reviews: Mutex::new(Vec::new()),
            fail_review: Mutex::new(None),
        })
    }
}

#[async_trait::async_trait]
impl GithubSender for RecordingGithub {
    async fn post_comment(
        &self,
        repo: &str,
        issue_number: i64,
        text: &str,
    ) -> Result<Option<ExternalRef>, GithubError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.posts
            .lock()
            .unwrap()
            .push((repo.into(), issue_number, text.into()));
        Ok(Some(ExternalRef::Github {
            repo: repo.into(),
            comment_id: id,
        }))
    }

    async fn update_comment(
        &self,
        _repo: &str,
        comment_id: i64,
        text: &str,
    ) -> Result<(), GithubError> {
        self.updates.lock().unwrap().push((comment_id, text.into()));
        Ok(())
    }

    async fn list_issue_comments(
        &self,
        _repo: &str,
        _issue_number: i64,
    ) -> Result<Vec<GithubIssueComment>, GithubError> {
        Ok(vec![])
    }

    async fn create_review(
        &self,
        repo: &str,
        pull_number: i64,
        commit_id: &str,
        comments: &[GithubReviewComment],
    ) -> Result<(), GithubError> {
        self.reviews.lock().unwrap().push(ReviewCall {
            repo: repo.into(),
            pull: pull_number,
            commit_id: commit_id.into(),
            comments: comments.to_vec(),
        });
        if let Some(err) = self.fail_review.lock().unwrap().clone() {
            return Err(err);
        }
        Ok(())
    }
}

struct RecordingSlack {
    posts: Mutex<Vec<(String, String)>>,
}

impl RecordingSlack {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            posts: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl SlackSender for RecordingSlack {
    async fn post_message(
        &self,
        channel: &str,
        text: &str,
        _thread_ts: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError> {
        self.posts
            .lock()
            .unwrap()
            .push((channel.into(), text.into()));
        Ok(Some(ExternalRef::Slack {
            channel_id: channel.into(),
            ts: "1700000001.000100".into(),
        }))
    }

    async fn update_message(
        &self,
        _channel: &str,
        _ts: &str,
        _text: &str,
    ) -> Result<(), SlackError> {
        Ok(())
    }
}

struct Harness {
    store: Arc<dyn Store>,
    state: AppState,
    workspace_id: maidan_types::WorkspaceId,
    channel_id: maidan_types::ChannelId,
    thread_id: ThreadId,
    member_id: maidan_types::MemberId,
    github: Arc<RecordingGithub>,
    slack: Arc<RecordingSlack>,
}

async fn harness(name: &str) -> Harness {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    let github = RecordingGithub::new();
    let slack = RecordingSlack::new();
    state.attach_github_sender(github.clone());
    state.attach_slack_sender(slack.clone());

    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
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
            title: Some(name.into()),
        })
        .await
        .unwrap();
    Harness {
        store,
        state,
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        member_id: member.id,
        github,
        slack,
    }
}

fn fixture_github_only() -> Value {
    let mut result: Value = serde_json::from_str(FIXTURE).unwrap();
    result["deliver_to"] = json!([{ "surface": "github", "repo": "beatgig/bgv3", "pr": 3915 }]);
    result
}

async fn bless_github(h: &Harness) {
    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: h.workspace_id,
            surface: EgressSurface::Github,
            selector: "beatgig/bgv3".into(),
        })
        .await
        .unwrap();
}

async fn bless_slack(h: &Harness) {
    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: h.workspace_id,
            surface: EgressSurface::Slack,
            selector: "C0123ABCDEF".into(),
        })
        .await
        .unwrap();
}

async fn set_result(h: &Harness, result: &Value) {
    let before = h
        .store
        .get_thread_result(h.thread_id)
        .await
        .unwrap()
        .map(|r| r.produced_at);
    loop {
        h.store
            .set_thread_result(h.thread_id, h.member_id, result)
            .await
            .unwrap();
        let after = h
            .store
            .get_thread_result(h.thread_id)
            .await
            .unwrap()
            .unwrap()
            .produced_at;
        if before.is_none_or(|b| after > b) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
}

async fn route(h: &Harness, log_id: i64) {
    notification_router::route_event(
        &h.state,
        log_id,
        &Event::ThreadResultSet {
            occurred_at: Utc::now(),
            workspace_id: h.workspace_id,
            channel_id: h.channel_id,
            thread_id: h.thread_id,
            produced_by: h.member_id,
        },
    )
    .await
    .unwrap();
}

async fn sweep(h: &Harness) {
    let _ = egress_worker::sweep_once(&h.state).await;
}

async fn delivered_row(h: &Harness) -> maidan_types::ResultDelivery {
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    let github: Vec<_> = rows.into_iter().filter(|r| r.surface == "github").collect();
    assert_eq!(github.len(), 1, "expected one github delivery row");
    github.into_iter().next().unwrap()
}

fn envelope(status: &str, deliver_to: Value, head_sha: Option<&str>, findings: Value) -> Value {
    let mut v = json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": "pi.review.result/1",
        "status": status,
        "deliver_to": deliver_to,
        "rendered": "## Findings",
        "summary": "1 finding",
        "view_in_pi": "https://pi.test/r/1",
        "findings": findings,
    });
    if let Some(sha) = head_sha {
        v["head_sha"] = json!(sha);
    }
    v
}

fn github_target() -> Value {
    json!([{ "surface": "github", "repo": "beatgig/bgv3", "pr": 3915 }])
}

fn slack_target() -> Value {
    json!([{ "surface": "slack", "channel": "C0123ABCDEF" }])
}

fn usable_findings() -> Value {
    json!([
        {
            "file": "auth.py",
            "body": "bypass @octocat",
            "line_range": { "start": 2, "end": 4 }
        },
        {
            "file": "auth.py",
            "body": "one line",
            "line_range": { "start": 7, "end": 7 }
        }
    ])
}

#[tokio::test]
async fn a_reviewed_github_result_posts_the_summary_and_a_right_side_inline_review() {
    let h = harness("inline-fixture").await;
    bless_github(&h).await;
    set_result(&h, &fixture_github_only()).await;
    route(&h, 1).await;
    sweep(&h).await;

    {
        let posts = h.github.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, "beatgig/bgv3");
        assert_eq!(posts[0].1, 3915);
        assert!(
            egress_body::comment_carries_result_marker(&posts[0].2, h.thread_id),
            "379 summary marker stays on the issue comment"
        );
    }

    {
        let reviews = h.github.reviews.lock().unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].repo, "beatgig/bgv3");
        assert_eq!(reviews[0].pull, 3915);
        assert_eq!(
            reviews[0].commit_id, HEAD_SHA,
            "commit_id is envelope head_sha — never a live PR head"
        );
        assert_eq!(reviews[0].comments.len(), 2);

        let first = &reviews[0].comments[0];
        assert_eq!(first.path, "auth.py");
        assert_eq!(first.line, 4, "GitHub line is line_range.end");
        assert_eq!(first.start_line, Some(2));
        assert_eq!(first.side, GithubDiffSide::Right);
        assert_eq!(first.side.as_str(), "RIGHT");
        assert!(
            !first.body.contains("<!-- maidan:result:"),
            "inline comments must not carry the 379 marker: {}",
            first.body
        );
        assert!(
            first.body.contains("Authentication is fully bypassed"),
            "the comment body is the producer's finding body"
        );

        let second = &reviews[0].comments[1];
        assert_eq!(second.line, 4);
        assert_eq!(second.start_line, Some(1));
        assert_eq!(second.side.as_str(), "RIGHT");
    }

    let delivered = delivered_row(&h).await;
    assert_eq!(delivered.status, status::DELIVERED);
    assert_eq!(delivered.external_ref.as_deref(), Some("1"));
}

#[tokio::test]
async fn missing_head_sha_posts_the_summary_and_skips_the_review() {
    let h = harness("no-sha").await;
    bless_github(&h).await;
    set_result(
        &h,
        &envelope("reviewed", github_target(), None, usable_findings()),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    assert_eq!(h.github.posts.lock().unwrap().len(), 1);
    assert!(
        h.github.reviews.lock().unwrap().is_empty(),
        "no commit_id ⇒ no review"
    );
    assert_eq!(delivered_row(&h).await.status, status::DELIVERED);
}

#[tokio::test]
async fn unusable_findings_post_the_summary_and_skip_the_review() {
    let h = harness("bad-findings").await;
    bless_github(&h).await;
    set_result(
        &h,
        &envelope(
            "reviewed",
            github_target(),
            Some(HEAD_SHA),
            json!([
                { "severity": "critical" },
                {
                    "file": "auth.py",
                    "body": "zero is not a line",
                    "line_range": { "start": 0, "end": 1 }
                }
            ]),
        ),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    assert_eq!(h.github.posts.lock().unwrap().len(), 1);
    assert!(h.github.reviews.lock().unwrap().is_empty());
    assert_eq!(delivered_row(&h).await.status, status::DELIVERED);
}

#[tokio::test]
async fn a_mention_in_a_finding_body_is_defused_on_the_inline_comment() {
    let h = harness("defuse").await;
    bless_github(&h).await;
    set_result(
        &h,
        &envelope(
            "reviewed",
            github_target(),
            Some(HEAD_SHA),
            usable_findings(),
        ),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    let reviews = h.github.reviews.lock().unwrap();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].comments[0].body, "bypass `@octocat`");
    assert_eq!(reviews[0].comments[0].side.as_str(), "RIGHT");
    assert_eq!(reviews[0].comments[1].start_line, None);
}

#[tokio::test]
async fn slack_only_delivery_never_creates_a_github_review() {
    let h = harness("slack-only").await;
    bless_slack(&h).await;
    set_result(
        &h,
        &envelope(
            "reviewed",
            slack_target(),
            Some(HEAD_SHA),
            usable_findings(),
        ),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    assert_eq!(h.slack.posts.lock().unwrap().len(), 1);
    assert!(h.github.posts.lock().unwrap().is_empty());
    assert!(h.github.reviews.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_non_reviewed_result_posts_a_failure_notice_and_no_review() {
    let h = harness("failed-status").await;
    bless_github(&h).await;
    set_result(
        &h,
        &envelope("failed", github_target(), Some(HEAD_SHA), usable_findings()),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    {
        let posts = h.github.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert!(posts[0].2.contains("status `failed`"));
        assert!(!posts[0].2.contains("bypass"));
    }
    assert!(
        h.github.reviews.lock().unwrap().is_empty(),
        "findings must not ride a non-reviewed delivery"
    );
    assert_eq!(delivered_row(&h).await.status, status::DELIVERED);
}

#[tokio::test]
async fn a_review_422_leaves_the_summary_delivered() {
    let h = harness("review-422").await;
    bless_github(&h).await;
    *h.github.fail_review.lock().unwrap() = Some(GithubError::Api {
        status: 422,
        rate_limited: false,
    });
    set_result(
        &h,
        &envelope(
            "reviewed",
            github_target(),
            Some(HEAD_SHA),
            usable_findings(),
        ),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    assert_eq!(h.github.posts.lock().unwrap().len(), 1);
    assert_eq!(
        h.github.reviews.lock().unwrap().len(),
        1,
        "the worker still attempts the review so the payload is what GitHub 422'd"
    );
    assert_eq!(h.github.reviews.lock().unwrap()[0].commit_id, HEAD_SHA);
    assert_eq!(delivered_row(&h).await.status, status::DELIVERED);
}

#[tokio::test]
async fn a_review_404_on_a_non_pr_issue_leaves_the_summary_delivered() {
    let h = harness("review-404").await;
    bless_github(&h).await;
    *h.github.fail_review.lock().unwrap() = Some(GithubError::Api {
        status: 404,
        rate_limited: false,
    });
    set_result(
        &h,
        &envelope(
            "reviewed",
            github_target(),
            Some(HEAD_SHA),
            usable_findings(),
        ),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    assert_eq!(h.github.posts.lock().unwrap().len(), 1);
    assert_eq!(delivered_row(&h).await.status, status::DELIVERED);
}

#[tokio::test]
async fn a_rereview_updates_the_summary_and_posts_a_new_comment_review() {
    let h = harness("rereview").await;
    bless_github(&h).await;
    set_result(
        &h,
        &envelope(
            "reviewed",
            github_target(),
            Some(HEAD_SHA),
            usable_findings(),
        ),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    let sha2 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    set_result(
        &h,
        &envelope(
            "reviewed",
            github_target(),
            Some(sha2),
            json!([{
                "file": "b.rs",
                "body": "new finding",
                "line_range": { "start": 9, "end": 9 }
            }]),
        ),
    )
    .await;
    route(&h, 2).await;
    sweep(&h).await;

    assert_eq!(
        h.github.posts.lock().unwrap().len(),
        1,
        "the 379 summary still updates in place"
    );
    assert_eq!(h.github.updates.lock().unwrap().len(), 1);
    let reviews = h.github.reviews.lock().unwrap();
    assert_eq!(
        reviews.len(),
        2,
        "each result posts a COMMENT review on its own head_sha"
    );
    assert_eq!(reviews[0].commit_id, HEAD_SHA);
    assert_eq!(reviews[1].commit_id, sha2);
    assert_eq!(reviews[1].comments[0].path, "b.rs");
    assert_eq!(reviews[1].comments[0].line, 9);
    assert_eq!(reviews[1].comments[0].start_line, None);
    assert_eq!(reviews[1].comments[0].side.as_str(), "RIGHT");
}
