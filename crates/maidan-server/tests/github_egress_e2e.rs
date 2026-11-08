//! GitHub projector egress. A Maidan message in a linked thread is relayed as a
//! GitHub issue/PR comment; a GitHub-sourced message (metadata tag) is not
//! echoed back (loop prevention); an unlinked thread is ignored.
//!
//! The relay is durable: `route_message_to_github` enqueues
//! and the egress worker posts, so each case sweeps the queue before asserting.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    egress_worker::sweep_once,
    github::{route_message_to_github, GithubError, GithubSender},
    AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EgressKind, EgressTarget, ExternalRef, MemberKind, NewChannel, NewEgressOutbox,
    NewGithubIssueLink, NewMember, NewMessage, NewThread, NewWorkspace,
};
use serde_json::json;

struct MockSender {
    sent: Mutex<Vec<(String, i64, String)>>,
}

#[async_trait::async_trait]
impl GithubSender for MockSender {
    async fn post_comment(
        &self,
        repo: &str,
        issue_number: i64,
        text: &str,
    ) -> Result<Option<ExternalRef>, GithubError> {
        let mut sent = self.sent.lock().unwrap();
        sent.push((repo.into(), issue_number, text.into()));
        Ok(Some(ExternalRef::Github {
            repo: repo.into(),
            comment_id: sent.len() as i64,
        }))
    }

    async fn update_comment(
        &self,
        _repo: &str,
        _comment_id: i64,
        _text: &str,
    ) -> Result<(), GithubError> {
        unreachable!("the projector egress never updates; that is result delivery")
    }

    async fn list_issue_comments(
        &self,
        _repo: &str,
        _issue_number: i64,
    ) -> Result<Vec<maidan_server::github::GithubIssueComment>, GithubError> {
        unreachable!("the projector egress never lists comments; that is result delivery")
    }

    async fn create_check_run(
        &self,
        _repo: &str,
        _check: &maidan_types::GithubCheckRun,
    ) -> Result<(), GithubError> {
        unreachable!("the projector egress never creates a check run; that is result delivery")
    }

    async fn create_review(
        &self,
        _repo: &str,
        _pull_number: i64,
        _commit_id: &str,
        _comments: &[maidan_types::GithubReviewComment],
    ) -> Result<(), GithubError> {
        unreachable!("the projector egress never creates a review; that is result delivery")
    }
}

async fn setup() -> (AppState, Arc<MockSender>, Arc<dyn Store>) {
    let sender = Arc::new(MockSender {
        sent: Mutex::new(Vec::new()),
    });
    let (state, store) = state_over(sender.clone()).await;
    (state, sender, store)
}

async fn state_over(sender: Arc<dyn GithubSender>) -> (AppState, Arc<dyn Store>) {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    std::mem::forget(dir);
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    state.attach_github_sender(sender);
    (state, store)
}

async fn post(
    store: &dyn Store,
    thread: maidan_types::ThreadId,
    author: maidan_types::MemberId,
    body: &str,
    metadata: serde_json::Value,
) -> maidan_types::Message {
    store
        .post_message_with_event(
            NewMessage {
                thread_id: thread,
                author_id: author,
                body: body.into(),
                metadata,
                content: None,
            },
            None,
        )
        .await
        .unwrap()
        .0
}

#[tokio::test]
async fn egress_relays_a_linked_thread_message_and_skips_github_sourced() {
    let (state, sender, store) = setup().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let agent = store
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
            name: "eng".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("issue-42".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .link_github_issue(NewGithubIssueLink {
            repo: "o/r".into(),
            issue_number: 42,
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: agent.id,
        })
        .await
        .unwrap();

    // A normal Maidan message in the linked thread is relayed as a comment.
    let m = post(
        store.as_ref(),
        thread.id,
        agent.id,
        "shipping it",
        json!({}),
    )
    .await;
    route_message_to_github(&state, 1, thread.id, &m).await;
    assert_eq!(
        sweep_once(&state).await,
        maidan_server::egress_worker::EgressSweepStats {
            sent: 1,
            ..Default::default()
        }
    );
    {
        let sent = sender.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], ("o/r".to_string(), 42, "shipping it".to_string()));
    }

    // A second replica routing the same event enqueues nothing new — one comment.
    route_message_to_github(&state, 1, thread.id, &m).await;
    sweep_once(&state).await;
    assert_eq!(
        sender.sent.lock().unwrap().len(),
        1,
        "the same event routed twice delivers once"
    );

    // A GitHub-sourced message (metadata tag) is NOT echoed back — no loop.
    let from_gh = post(
        store.as_ref(),
        thread.id,
        agent.id,
        "octocat: hi",
        json!({ "github": { "user": "octocat", "repo": "o/r", "issue": 42 } }),
    )
    .await;
    route_message_to_github(&state, 2, thread.id, &from_gh).await;
    sweep_once(&state).await;
    assert_eq!(
        sender.sent.lock().unwrap().len(),
        1,
        "a GitHub-sourced message is not relayed back to GitHub"
    );

    // A message in an unlinked thread is ignored.
    let other = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("other".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let m2 = post(store.as_ref(), other.id, agent.id, "unlinked", json!({})).await;
    route_message_to_github(&state, 3, other.id, &m2).await;
    sweep_once(&state).await;
    assert_eq!(
        sender.sent.lock().unwrap().len(),
        1,
        "an unlinked thread does not relay to GitHub"
    );
}

/// A sender GitHub answers with a secondary rate limit on every post.
struct RateLimitedSender {
    posts: AtomicUsize,
}

#[async_trait::async_trait]
impl GithubSender for RateLimitedSender {
    async fn post_comment(
        &self,
        _repo: &str,
        _issue_number: i64,
        _text: &str,
    ) -> Result<Option<ExternalRef>, GithubError> {
        self.posts.fetch_add(1, Ordering::SeqCst);
        Err(GithubError::Api {
            status: 403,
            rate_limited: true,
            detail: "You have exceeded a secondary rate limit".into(),
            retry_after: Some(900),
        })
    }

    async fn update_comment(&self, _: &str, _: i64, _: &str) -> Result<(), GithubError> {
        unreachable!("the projector egress never updates")
    }

    async fn list_issue_comments(
        &self,
        _: &str,
        _: i64,
    ) -> Result<Vec<maidan_server::github::GithubIssueComment>, GithubError> {
        unreachable!("the projector egress never lists comments")
    }

    async fn create_check_run(
        &self,
        _: &str,
        _: &maidan_types::GithubCheckRun,
    ) -> Result<(), GithubError> {
        unreachable!("the projector egress never creates a check run")
    }

    async fn create_review(
        &self,
        _: &str,
        _: i64,
        _: &str,
        _: &[maidan_types::GithubReviewComment],
    ) -> Result<(), GithubError> {
        unreachable!("the projector egress never creates a review")
    }
}

/// The 2026-11-08 swarm rows retried a secondary rate limit on the 30s
/// backoff and failed four times inside GitHub's window. A rate-limited 403
/// is a retry, never a disabled link, and the retry waits out `retry-after`.
#[tokio::test]
async fn a_rate_limited_post_is_retried_no_sooner_than_githubs_retry_after() {
    let sender = Arc::new(RateLimitedSender {
        posts: AtomicUsize::new(0),
    });
    let (state, store) = state_over(sender.clone()).await;
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "eng".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("issue-42".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: ws.id,
            thread_id: thread.id,
            source_log_id: 1,
            target: EgressTarget::Github {
                repo: "o/r".into(),
                issue_number: 42,
            },
            body: "shipping it".into(),
            kind: EgressKind::Projector,
        })
        .await
        .unwrap();

    let before = Utc::now();
    assert_eq!(
        sweep_once(&state).await,
        maidan_server::egress_worker::EgressSweepStats {
            retried: 1,
            ..Default::default()
        },
        "a rate limit is retried, not dead-lettered or disabled"
    );
    assert_eq!(sender.posts.load(Ordering::SeqCst), 1);
    // The first backoff alone is 30s; ten minutes on, it would be due.
    assert!(
        store
            .claim_next_due_egress(before + chrono::Duration::seconds(600), 60)
            .await
            .unwrap()
            .is_none(),
        "rescheduled inside GitHub's retry-after window"
    );
    assert!(
        store
            .claim_next_due_egress(Utc::now() + chrono::Duration::seconds(901), 60)
            .await
            .unwrap()
            .is_some(),
        "due once the retry-after window has passed"
    );
}
