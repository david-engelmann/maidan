//! Cluster 379.4: idempotent update-in-place for result delivery.
//!
//! A re-review of the same thread edits the object the first delivery created
//! (a GitHub comment, a Slack message) rather than stacking a second one. The
//! stored `external_ref` is the happy path; a hidden
//! `<!-- maidan:result:<thread_id> -->` marker at byte 0 of the GitHub body is
//! the recovery path if that handle is lost. Slack has no equivalent marker
//! and re-posts. Projector rows never take this path — they keep posting.

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
    status, EgressSurface, Event, ExternalRef, MemberKind, NewChannel, NewEgressTarget,
    NewGithubIssueLink, NewMember, NewThread, NewWorkspace, ThreadId, WAITER_RESULT_SCHEMA,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct GhComment {
    repo: String,
    issue: i64,
    id: i64,
    body: String,
}

struct RecordingGithub {
    next_id: AtomicI64,
    comments: Mutex<Vec<GhComment>>,
    posts: Mutex<Vec<(String, i64, String)>>,
    updates: Mutex<Vec<(i64, String)>>,
    lists: Mutex<usize>,
    fail_post: Mutex<Option<GithubError>>,
}

impl RecordingGithub {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            next_id: AtomicI64::new(1),
            comments: Mutex::new(Vec::new()),
            posts: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
            lists: Mutex::new(0),
            fail_post: Mutex::new(None),
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
        if let Some(err) = self.fail_post.lock().unwrap().clone() {
            return Err(err);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.posts
            .lock()
            .unwrap()
            .push((repo.into(), issue_number, text.into()));
        self.comments.lock().unwrap().push(GhComment {
            repo: repo.into(),
            issue: issue_number,
            id,
            body: text.into(),
        });
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
        let mut comments = self.comments.lock().unwrap();
        let Some(found) = comments.iter_mut().find(|c| c.id == comment_id) else {
            return Err(GithubError::Api {
                status: 404,
                rate_limited: false,
            });
        };
        found.body = text.into();
        self.updates.lock().unwrap().push((comment_id, text.into()));
        Ok(())
    }

    async fn list_issue_comments(
        &self,
        repo: &str,
        issue_number: i64,
    ) -> Result<Vec<GithubIssueComment>, GithubError> {
        *self.lists.lock().unwrap() += 1;
        Ok(self
            .comments
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.repo == repo && c.issue == issue_number)
            .map(|c| GithubIssueComment {
                id: c.id,
                body: c.body.clone(),
            })
            .collect())
    }

    async fn create_review(
        &self,
        _repo: &str,
        _pull_number: i64,
        _commit_id: &str,
        _comments: &[maidan_types::GithubReviewComment],
    ) -> Result<(), GithubError> {
        // 379.4 envelopes have no findings; Cluster 380.2 e2e covers reviews.
        Ok(())
    }
}

struct RecordingSlack {
    posts: Mutex<Vec<(String, String)>>,
    updates: Mutex<Vec<(String, String, String)>>,
}

impl RecordingSlack {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            posts: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl SlackSender for RecordingSlack {
    async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError> {
        assert_eq!(thread_ts, None, "result delivery posts top-level on Slack");
        let mut posts = self.posts.lock().unwrap();
        posts.push((channel.into(), text.into()));
        Ok(Some(ExternalRef::Slack {
            channel_id: channel.into(),
            ts: format!("17000000{:02}.000100", posts.len()),
        }))
    }

    async fn update_message(&self, channel: &str, ts: &str, text: &str) -> Result<(), SlackError> {
        if ts == "gone" {
            return Err(SlackError::Api("message_not_found".into()));
        }
        self.updates
            .lock()
            .unwrap()
            .push((channel.into(), ts.into(), text.into()));
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

fn envelope(status: &str, deliver_to: Value, rendered: &str, summary: &str) -> Value {
    json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": "example.review.result/1",
        "status": status,
        "deliver_to": deliver_to,
        "rendered": rendered,
        "summary": summary,
        "view_url": "https://producer.example.test/r/1",
        "pr": "acme/widgets#7",
    })
}

fn github_target() -> Value {
    json!([{ "surface": "github", "repo": "acme/widgets", "pr": 7 }])
}

fn slack_target() -> Value {
    json!([{ "surface": "slack", "channel": "C0123ABCDEF" }])
}

async fn bless_github(h: &Harness) {
    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: h.workspace_id,
            surface: EgressSurface::Github,
            selector: "acme/widgets".into(),
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

async fn row(h: &Harness) -> maidan_types::ResultDelivery {
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows.len(), 1, "expected one delivery row, got {rows:?}");
    rows.into_iter().next().unwrap()
}

#[tokio::test]
async fn a_first_github_delivery_posts_a_marked_comment_and_stores_the_ref() {
    let h = harness("first-gh").await;
    bless_github(&h).await;
    set_result(
        &h,
        &envelope("reviewed", github_target(), "first review", "one finding"),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    {
        let posts = h.github.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert!(
            egress_body::comment_carries_result_marker(&posts[0].2, h.thread_id),
            "marker at byte 0: {}",
            &posts[0].2[..posts[0].2.len().min(80)]
        );
        assert!(posts[0].2.contains("first review"));
    }
    assert!(h.github.updates.lock().unwrap().is_empty());
    assert_eq!(
        *h.github.lists.lock().unwrap(),
        0,
        "first post does not list"
    );

    let delivered = row(&h).await;
    assert_eq!(delivered.status, status::DELIVERED);
    assert_eq!(delivered.external_ref.as_deref(), Some("1"));
    assert!(delivered.delivered_revision.is_some());
}

#[tokio::test]
async fn a_rereview_updates_the_same_github_comment() {
    let h = harness("rereview-gh").await;
    bless_github(&h).await;
    set_result(
        &h,
        &envelope("reviewed", github_target(), "first review", "one"),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    set_result(
        &h,
        &envelope("reviewed", github_target(), "second review", "two"),
    )
    .await;
    route(&h, 2).await;
    sweep(&h).await;

    assert_eq!(
        h.github.posts.lock().unwrap().len(),
        1,
        "a re-review must not stack a second comment"
    );
    {
        let updates = h.github.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, 1, "the first comment id is what we edit");
        assert!(updates[0].1.contains("second review"));
        assert!(egress_body::comment_carries_result_marker(
            &updates[0].1,
            h.thread_id
        ));
    }

    let delivered = row(&h).await;
    assert_eq!(delivered.status, status::DELIVERED);
    assert_eq!(delivered.external_ref.as_deref(), Some("1"));
}

#[tokio::test]
async fn a_lost_github_ref_recovers_via_the_marker_at_byte_zero() {
    let h = harness("recover-gh").await;
    bless_github(&h).await;
    set_result(
        &h,
        &envelope("reviewed", github_target(), "first review", "one"),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    let first = row(&h).await;
    h.store
        .mark_result_delivered(first.id, None, first.armed_revision)
        .await
        .unwrap();
    assert!(
        h.store.list_result_deliveries(h.thread_id).await.unwrap()[0]
            .external_ref
            .is_none()
    );

    set_result(
        &h,
        &envelope("reviewed", github_target(), "recovered review", "two"),
    )
    .await;
    route(&h, 2).await;
    sweep(&h).await;

    assert_eq!(
        h.github.posts.lock().unwrap().len(),
        1,
        "recovery must PATCH, not POST"
    );
    assert!(
        *h.github.lists.lock().unwrap() >= 1,
        "lost ref lists comments to find the marker"
    );
    {
        let updates = h.github.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, 1);
        assert!(updates[0].1.contains("recovered review"));
    }

    let delivered = row(&h).await;
    assert_eq!(delivered.external_ref.as_deref(), Some("1"));
    assert_eq!(delivered.status, status::DELIVERED);
}

#[tokio::test]
async fn an_update_404_recovers_via_the_marker_instead_of_disabling_a_link() {
    let h = harness("update-404").await;
    bless_github(&h).await;
    h.store
        .link_github_issue(NewGithubIssueLink {
            repo: "acme/widgets".into(),
            issue_number: 7,
            workspace_id: h.workspace_id,
            channel_id: h.channel_id,
            thread_id: h.thread_id,
            member_id: h.member_id,
        })
        .await
        .unwrap();

    set_result(
        &h,
        &envelope("reviewed", github_target(), "first review", "one"),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    let first = row(&h).await;
    // A stale handle that GitHub will 404.
    h.store
        .mark_result_delivered(first.id, Some("999999"), first.armed_revision)
        .await
        .unwrap();

    set_result(
        &h,
        &envelope("reviewed", github_target(), "after 404", "two"),
    )
    .await;
    route(&h, 2).await;
    sweep(&h).await;

    assert_eq!(h.github.posts.lock().unwrap().len(), 1);
    {
        let updates = h.github.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, 1, "recovered the real comment, not 999999");
        assert!(updates[0].1.contains("after 404"));
    }

    let link = h
        .store
        .get_github_issue_link("acme/widgets", 7)
        .await
        .unwrap()
        .expect("link");
    assert!(
        link.disabled_at.is_none(),
        "a result-delivery 404 must not disable a projector issue-link"
    );
}

#[tokio::test]
async fn a_result_401_dead_letters_without_disabling_the_projector_link() {
    let h = harness("result-401").await;
    bless_github(&h).await;
    h.store
        .link_github_issue(NewGithubIssueLink {
            repo: "acme/widgets".into(),
            issue_number: 7,
            workspace_id: h.workspace_id,
            channel_id: h.channel_id,
            thread_id: h.thread_id,
            member_id: h.member_id,
        })
        .await
        .unwrap();
    *h.github.fail_post.lock().unwrap() = Some(GithubError::Api {
        status: 401,
        rate_limited: false,
    });

    set_result(
        &h,
        &envelope("reviewed", github_target(), "will not post", "nope"),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    assert!(h.github.posts.lock().unwrap().is_empty());
    let delivered = row(&h).await;
    assert_eq!(delivered.status, status::FAILED);
    assert!(delivered.external_ref.is_none());

    let link = h
        .store
        .get_github_issue_link("acme/widgets", 7)
        .await
        .unwrap()
        .expect("link");
    assert!(
        link.disabled_at.is_none(),
        "result 401 is not a projector-link failure"
    );
}

#[tokio::test]
async fn a_rereview_updates_the_same_slack_message() {
    let h = harness("rereview-slack").await;
    bless_slack(&h).await;
    set_result(
        &h,
        &envelope("reviewed", slack_target(), "## unused", "first summary"),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    set_result(
        &h,
        &envelope("reviewed", slack_target(), "## unused", "second summary"),
    )
    .await;
    route(&h, 2).await;
    sweep(&h).await;

    assert_eq!(h.slack.posts.lock().unwrap().len(), 1);
    {
        let updates = h.slack.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1, "1700000001.000100");
        assert!(updates[0].2.contains("second summary"));
    }

    let delivered = row(&h).await;
    assert_eq!(delivered.external_ref.as_deref(), Some("1700000001.000100"));
}

#[tokio::test]
async fn a_lost_slack_ref_posts_again_there_is_no_marker() {
    let h = harness("lost-slack").await;
    bless_slack(&h).await;
    set_result(
        &h,
        &envelope("reviewed", slack_target(), "## unused", "first summary"),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;

    let first = row(&h).await;
    h.store
        .mark_result_delivered(first.id, None, first.armed_revision)
        .await
        .unwrap();

    set_result(
        &h,
        &envelope("reviewed", slack_target(), "## unused", "second summary"),
    )
    .await;
    route(&h, 2).await;
    sweep(&h).await;

    assert_eq!(
        h.slack.posts.lock().unwrap().len(),
        2,
        "Slack has no recovery marker; a lost ref posts again"
    );
    assert!(h.slack.updates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_projector_row_to_the_same_issue_still_posts_and_never_updates() {
    // The kind discriminator: a projector MessagePosted aimed at the same
    // GitHub issue must not PATCH the result comment.
    let h = harness("kind-split").await;
    bless_github(&h).await;
    set_result(
        &h,
        &envelope("reviewed", github_target(), "the review", "one"),
    )
    .await;
    route(&h, 1).await;
    sweep(&h).await;
    assert_eq!(h.github.posts.lock().unwrap().len(), 1);

    h.store
        .enqueue_egress(maidan_types::NewEgressOutbox {
            workspace_id: h.workspace_id,
            thread_id: h.thread_id,
            source_log_id: 99,
            target: maidan_types::EgressTarget::Github {
                repo: "acme/widgets".into(),
                issue_number: 7,
            },
            body: "a projector echo".into(),
            kind: maidan_types::EgressKind::Projector,
        })
        .await
        .unwrap();
    sweep(&h).await;

    assert_eq!(
        h.github.posts.lock().unwrap().len(),
        2,
        "the projector posts its own comment"
    );
    assert!(
        h.github.updates.lock().unwrap().is_empty(),
        "the projector path never updates"
    );
    assert_eq!(h.github.posts.lock().unwrap()[1].2, "a projector echo");
}
