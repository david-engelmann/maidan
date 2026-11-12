//! The credential guard end to end: a result routed through the real
//! notification router and egress worker, with a recording GitHub sender.
//! A result whose text holds a credential shape posts nothing (no comment,
//! no inline review, no check run), is recorded as a skip that names the
//! kind and never the text, and stays unsent on a replay. A clean result
//! still posts.
//!
//! Every credential is assembled at run time, so this file holds no string a
//! secrets scanner reads as a live key.

use maidan_store::test_support::SetResultClaiming;
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc, Mutex,
};

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    egress_worker, github::GithubSender, notification_router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    status, EgressSurface, Event, ExternalRef, MemberId, MemberKind, NewChannel, NewEgressTarget,
    NewMember, NewThread, NewWorkspace, ResultDelivery, ThreadId, WorkspaceId,
    WAITER_RESULT_SCHEMA,
};
use serde_json::{json, Value};

const REPO: &str = "acme/widgets";
const HEAD_SHA: &str = "0123456789abcdef0123456789abcdef01234567";

#[derive(Default)]
struct RecordingGithub {
    next_id: AtomicI64,
    posts: Mutex<Vec<String>>,
    reviews: Mutex<Vec<String>>,
    checks: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl GithubSender for RecordingGithub {
    async fn post_comment(
        &self,
        _repo: &str,
        _issue_number: i64,
        text: &str,
    ) -> Result<Option<ExternalRef>, maidan_server::github::GithubError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.posts.lock().unwrap().push(text.into());
        Ok(Some(ExternalRef::Github {
            repo: REPO.into(),
            comment_id: id,
        }))
    }

    async fn update_comment(
        &self,
        _repo: &str,
        _comment_id: i64,
        text: &str,
    ) -> Result<(), maidan_server::github::GithubError> {
        self.posts.lock().unwrap().push(text.into());
        Ok(())
    }

    async fn list_issue_comments(
        &self,
        _repo: &str,
        _issue_number: i64,
    ) -> Result<Vec<maidan_server::github::GithubIssueComment>, maidan_server::github::GithubError>
    {
        Ok(vec![])
    }

    async fn create_check_run(
        &self,
        _repo: &str,
        check: &maidan_types::GithubCheckRun,
    ) -> Result<(), maidan_server::github::GithubError> {
        self.checks.lock().unwrap().push(check.summary.clone());
        Ok(())
    }

    fn posts_check_runs(&self) -> bool {
        true
    }

    async fn create_review(
        &self,
        _repo: &str,
        _pull_number: i64,
        _commit_id: &str,
        comments: &[maidan_types::GithubReviewComment],
    ) -> Result<(), maidan_server::github::GithubError> {
        let mut reviews = self.reviews.lock().unwrap();
        reviews.extend(comments.iter().map(|c| c.body.clone()));
        Ok(())
    }
}

struct Harness {
    store: Arc<dyn Store>,
    state: AppState,
    github: Arc<RecordingGithub>,
    workspace_id: WorkspaceId,
    channel_id: maidan_types::ChannelId,
    thread_id: ThreadId,
    member_id: MemberId,
}

impl Harness {
    fn posts(&self) -> Vec<String> {
        self.github.posts.lock().unwrap().clone()
    }

    fn reviews(&self) -> Vec<String> {
        self.github.reviews.lock().unwrap().clone()
    }

    fn checks(&self) -> Vec<String> {
        self.github.checks.lock().unwrap().clone()
    }
}

async fn harness(name: &str) -> Harness {
    let pool = maidan_search::sqlite_pool_options()
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
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
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .allow_egress_target(NewEgressTarget {
            workspace_id: ws.id,
            surface: EgressSurface::Github,
            selector: REPO.into(),
        })
        .await
        .unwrap();
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let github = Arc::new(RecordingGithub::default());
    state.attach_github_sender(github.clone());
    Harness {
        store,
        state,
        github,
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        member_id: member.id,
    }
}

fn envelope(rendered: &str, findings: Value) -> Value {
    json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": "example.review.result/1",
        "status": "reviewed",
        "deliver_to": [{ "surface": "github", "repo": REPO, "pr": 7 }],
        "rendered": rendered,
        "summary": "1 finding",
        "view_url": "https://producer.example.test/r/1",
        "head_sha": HEAD_SHA,
        "findings": findings,
    })
}

fn finding(body: &str) -> Value {
    json!([{ "file": "src/auth.rs", "body": body, "line_range": { "start": 2, "end": 2 } }])
}

async fn route(h: &Harness, result: &Value, log_id: i64) {
    h.store
        .set_thread_result_claiming(h.thread_id, h.member_id, result)
        .await
        .unwrap();
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

async fn github_row(h: &Harness) -> ResultDelivery {
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    rows.into_iter()
        .find(|r| r.surface == "github")
        .expect("a github delivery row")
}

/// A token of the shape GitHub mints, built here rather than written down.
fn github_token() -> String {
    format!("ghp_{}", "aB3dE5gH7jK9".repeat(3))
}

#[tokio::test]
async fn a_result_whose_comment_holds_a_github_token_posts_nothing_and_records_why() {
    let h = harness("leaky-comment").await;
    let token = github_token();
    let rendered = format!("## Findings\nThe config loads `{token}` at boot.");
    route(&h, &envelope(&rendered, finding("hard-coded secret")), 1).await;

    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!((stats.sent, stats.refused), (0, 1), "{stats:?}");
    assert!(h.posts().is_empty(), "no comment: {:?}", h.posts());
    assert!(h.reviews().is_empty(), "no inline review");
    assert!(h.checks().is_empty(), "no check run");

    let row = github_row(&h).await;
    assert_eq!(row.status, status::SKIPPED);
    let reason = row.last_error.clone().unwrap_or_default();
    assert_eq!(
        reason,
        "refused: the comment holds what looks like a GitHub token (gh*_…); nothing was sent to GitHub"
    );
    assert!(!reason.contains(&token));
    let audit = serde_json::to_string(&h.store.list_audit(50).await.unwrap()).unwrap();
    assert!(audit.contains("refused"), "the attempt is audited: {audit}");
    assert!(!audit.contains(&token), "the audit trail holds no token");

    // A replay rebuilds the body from the stored result, and the guard sees
    // it again: retrying never gets the token out.
    let _ = maidan_store::replay_result_delivery(
        h.store.as_ref(),
        h.workspace_id,
        h.thread_id,
        row.id,
        String::new(),
    )
    .await
    .unwrap();
    egress_worker::sweep_once(&h.state).await;
    assert!(h.posts().is_empty(), "the replay posts nothing either");
    let again = github_row(&h).await;
    assert_eq!(again.status, status::SKIPPED);
    assert!(!again.last_error.unwrap_or_default().contains(&token));
}

#[tokio::test]
async fn a_credential_in_an_inline_finding_alone_refuses_the_summary_too() {
    let h = harness("leaky-finding").await;
    let key = format!("AKIA{}", "Q2W3E4R5T6Y7U8I9");
    let body = format!("the deploy script exports {key}");
    route(
        &h,
        &envelope("## Findings\nOne finding.", finding(&body)),
        1,
    )
    .await;

    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!(stats.refused, 1, "{stats:?}");
    assert!(h.posts().is_empty(), "the summary does not post either");
    assert!(h.reviews().is_empty());
    assert!(h.checks().is_empty());
    let reason = github_row(&h).await.last_error.unwrap_or_default();
    assert!(
        reason.contains("an AWS access key id") && !reason.contains(&key),
        "{reason}"
    );
}

#[tokio::test]
async fn a_clean_result_still_posts_its_comment_review_and_check() {
    let h = harness("clean").await;
    route(
        &h,
        &envelope(
            "## Findings\nThe token is logged at debug; use `TokenSecret`.",
            finding("log the token's id, not the token"),
        ),
        1,
    )
    .await;

    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!((stats.sent, stats.refused), (1, 0), "{stats:?}");
    assert_eq!(h.posts().len(), 1);
    assert_eq!(h.reviews().len(), 1);
    assert_eq!(h.checks().len(), 1);
    assert_eq!(github_row(&h).await.status, status::DELIVERED);
}
