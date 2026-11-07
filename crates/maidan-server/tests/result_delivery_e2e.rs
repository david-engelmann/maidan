//! A `ThreadResultSet` fetches, parses, allowlist-checks, and enqueues (or
//! records a skip) per `deliver_to` target.
//!
//! Direct `route_event` — no worker-loop timing. The assertions are the
//! contract in `docs/Result Delivery.md`: empty `deliver_to` writes nothing; an
//! unblessed or unknown target is a recorded skip, never an error; a
//! non-`reviewed` status delivers a Maidan-authored notice built from `status`
//! alone; partial delivery is the model.

use maidan_store::test_support::SetResultClaiming;
use std::sync::Arc;

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    egress_worker,
    github::{GithubError, GithubIssueComment, GithubSender},
    linear::{LinearError, LinearIssue, LinearSender},
    notification_router,
    slack::{SlackError, SlackSender},
    AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    status, EgressSurface, Event, EventKind, ExternalRef, GithubCheckRun, GithubReviewComment,
    MemberKind, NewChannel, NewEgressTarget, NewMember, NewThread, NewWorkspace, ThreadId,
    WAITER_RESULT_SCHEMA,
};
use serde_json::{json, Value};

/// A Linear team id, the key the allowlist blesses.
const TEAM: &str = "6a1f1c4e-2f0b-4d5e-9a7b-0c1d2e3f4a5b";

/// A sender for every surface, so a blessed target is armed and queued: a
/// server with no sender for a surface fails the target at once instead.
/// Only the worker posts, and these tests drain the queue by hand, so
/// anything that reaches here is refused.
struct Unsent;

#[async_trait::async_trait]
impl SlackSender for Unsent {
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

/// A Slack that takes every post.
struct Posting;

#[async_trait::async_trait]
impl SlackSender for Posting {
    async fn post_message(
        &self,
        channel: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError> {
        Ok(Some(ExternalRef::Slack {
            channel_id: channel.into(),
            ts: "1700000001.000100".into(),
        }))
    }

    async fn update_message(&self, _: &str, _: &str, _: &str) -> Result<(), SlackError> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl GithubSender for Unsent {
    async fn post_comment(
        &self,
        _: &str,
        _: i64,
        _: &str,
    ) -> Result<Option<ExternalRef>, GithubError> {
        Err(GithubError::Http("these tests send nothing".into()))
    }

    async fn update_comment(&self, _: &str, _: i64, _: &str) -> Result<(), GithubError> {
        Err(GithubError::Http("these tests send nothing".into()))
    }

    async fn list_issue_comments(
        &self,
        _: &str,
        _: i64,
    ) -> Result<Vec<GithubIssueComment>, GithubError> {
        Err(GithubError::Http("these tests send nothing".into()))
    }

    async fn create_review(
        &self,
        _: &str,
        _: i64,
        _: &str,
        _: &[GithubReviewComment],
    ) -> Result<(), GithubError> {
        Err(GithubError::Http("these tests send nothing".into()))
    }

    async fn create_check_run(&self, _: &str, _: &GithubCheckRun) -> Result<(), GithubError> {
        Err(GithubError::Http("these tests send nothing".into()))
    }
}

#[async_trait::async_trait]
impl LinearSender for Unsent {
    async fn issue(&self, _: &str) -> Result<LinearIssue, LinearError> {
        Err(LinearError::Http("these tests send nothing".into()))
    }

    async fn create_comment(&self, _: &str, _: &str) -> Result<String, LinearError> {
        Err(LinearError::Http("these tests send nothing".into()))
    }

    async fn update_comment(&self, _: &str, _: &str) -> Result<(), LinearError> {
        Err(LinearError::Http("these tests send nothing".into()))
    }

    async fn move_issue(&self, _: &str, _: &str) -> Result<(), LinearError> {
        Err(LinearError::Http("these tests send nothing".into()))
    }
}

struct Harness {
    store: Arc<dyn Store>,
    state: AppState,
    workspace_id: maidan_types::WorkspaceId,
    channel_id: maidan_types::ChannelId,
    thread_id: ThreadId,
    member_id: maidan_types::MemberId,
}

/// A server with a sender for every surface.
async fn harness(name: &str) -> Harness {
    let mut h = harness_without_senders(name).await;
    h.state.attach_slack_sender(Arc::new(Unsent));
    h.state.attach_github_sender(Arc::new(Unsent));
    h.state.attach_linear_sender(Arc::new(Unsent));
    h
}

/// A server none of whose delivery credentials are set.
async fn harness_without_senders(name: &str) -> Harness {
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
    let state = AppState::for_tests(store.clone(), artifacts, bus, search);

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
    Harness {
        store,
        state,
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        member_id: member.id,
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

async fn set_and_route(h: &Harness, result: &Value, log_id: i64) {
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

async fn bless_github(h: &Harness, repo: &str) {
    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: h.workspace_id,
            surface: EgressSurface::Github,
            selector: repo.into(),
        })
        .await
        .unwrap();
}

async fn bless_slack(h: &Harness, channel: &str) {
    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: h.workspace_id,
            surface: EgressSurface::Slack,
            selector: channel.into(),
        })
        .await
        .unwrap();
}

async fn bless_linear(h: &Harness) {
    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: h.workspace_id,
            surface: EgressSurface::Linear,
            selector: TEAM.into(),
        })
        .await
        .unwrap();
}

/// The `result_delivery_failed` events in the thread's log, oldest first.
async fn failed_events(h: &Harness) -> Vec<Event> {
    h.store
        .list_events_matching(
            h.workspace_id,
            None,
            Some(h.thread_id),
            &[EventKind::ResultDeliveryFailed],
            0,
            100,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|stored| stored.opened_event().unwrap())
        .collect()
}

/// Route a result to `target` on a server with no sender for its surface,
/// then check the row failed naming `setting`, nothing was queued, the
/// result's roll-up is not `pending`, and the event stream heard it.
async fn assert_unconfigured_target_fails(h: &Harness, target: Value, surface: &str, reason: &str) {
    set_and_route(
        h,
        &envelope("reviewed", json!([target]), "rendered", "summary"),
        1,
    )
    .await;
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].surface, surface);
    assert_eq!(rows[0].status, status::FAILED, "{rows:?}");
    assert_eq!(rows[0].last_error.as_deref(), Some(reason));
    assert_eq!(maidan_server::dto::delivery_state(&rows), "failed");
    assert!(
        claim_body(h).await.is_none(),
        "nothing is queued for a surface no worker can send to"
    );
    let events = failed_events(h).await;
    assert_eq!(events.len(), 1, "{events:?}");
    let Event::ResultDeliveryFailed {
        surface: said_surface,
        status: said_status,
        last_error,
        ..
    } = &events[0]
    else {
        panic!("not a result_delivery_failed: {:?}", events[0]);
    };
    assert_eq!(said_surface, surface);
    assert_eq!(said_status, status::FAILED);
    assert_eq!(last_error.as_deref(), Some(reason));
}

async fn claim_body(h: &Harness) -> Option<String> {
    h.store
        .claim_next_due_egress(Utc::now(), 120)
        .await
        .unwrap()
        .map(|e| e.body)
}

#[tokio::test]
async fn an_empty_deliver_to_writes_zero_rows() {
    let h = harness("empty").await;
    set_and_route(
        &h,
        &envelope("reviewed", json!([]), "rendered", "summary"),
        1,
    )
    .await;
    assert!(
        h.store
            .list_result_deliveries(h.thread_id)
            .await
            .unwrap()
            .is_empty(),
        "empty deliver_to is valid and normal — delivered nowhere, no rows"
    );
    assert!(claim_body(&h).await.is_none(), "nothing was enqueued");
}

#[tokio::test]
async fn an_unrecognized_envelope_is_inert() {
    let h = harness("inert").await;
    set_and_route(
        &h,
        &json!({
            "schema": "maidan.waiter.result/2",
            "result_kind": "example.review.result/1",
            "status": "reviewed",
            "deliver_to": [{ "surface": "github", "repo": "acme/widgets", "pr": 7 }],
            "rendered": "must not be delivered",
        }),
        1,
    )
    .await;
    assert!(h
        .store
        .list_result_deliveries(h.thread_id)
        .await
        .unwrap()
        .is_empty());
    assert!(claim_body(&h).await.is_none());
}

#[tokio::test]
async fn a_missing_result_is_inert() {
    let h = harness("missing").await;
    // Event without a stored result — the pointer has nothing to fetch.
    notification_router::route_event(
        &h.state,
        1,
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
    assert!(h
        .store
        .list_result_deliveries(h.thread_id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_blessed_github_target_is_armed_and_enqueued() {
    let h = harness("blessed-gh").await;
    bless_github(&h, "acme/widgets").await;
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([{ "surface": "github", "repo": "acme/widgets", "pr": 7 }]),
            "ping @octocat about this",
            "3 findings",
        ),
        1,
    )
    .await;
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, status::PENDING);
    assert_eq!(rows[0].surface, "github");
    assert_eq!(rows[0].selector, "acme/widgets#7");
    let body = claim_body(&h).await.expect("enqueued");
    assert!(
        maidan_server::egress_body::comment_carries_result_marker(&body, h.thread_id),
        "github body carries the recovery marker at byte 0: {body:.80}"
    );
    assert!(
        body.contains("`@octocat`"),
        "github body is the defused rendered: {body}"
    );
    assert!(
        !body.contains("3 findings"),
        "github must not receive the slack summary: {body}"
    );
    assert!(body.contains("PR: acme/widgets#7"));
}

#[tokio::test]
async fn an_unblessed_target_is_skipped_not_enqueued() {
    let h = harness("unblessed").await;
    // Allowlist empty ⇒ deliver nowhere. A correct deliver_to can still
    // land nowhere, and that is a normal outcome, not a producer bug.
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([{ "surface": "github", "repo": "acme/widgets", "pr": 7 }]),
            "must not be posted",
            "must not be posted",
        ),
        1,
    )
    .await;
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, status::SKIPPED);
    assert_eq!(
        rows[0].last_error.as_deref(),
        Some("target not in the workspace egress allowlist")
    );
    assert!(claim_body(&h).await.is_none());
}

#[tokio::test]
async fn an_unknown_surface_is_skipped_with_a_recorded_warning() {
    let h = harness("unknown").await;
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([{ "surface": "discord", "webhook": "https://x.test/hook" }]),
            "must not be posted",
            "must not be posted",
        ),
        1,
    )
    .await;
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, status::SKIPPED);
    assert_eq!(rows[0].surface, "discord");
    assert_eq!(
        rows[0].last_error.as_deref(),
        Some("unknown surface 'discord'")
    );
    assert!(claim_body(&h).await.is_none());
}

#[tokio::test]
async fn a_slack_hash_name_is_unusable_and_skipped() {
    let h = harness("hash-name").await;
    // Blessing the name would not help: a #name is not an allowlist key, and
    // to_egress_target refuses it before the allowlist is consulted.
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([{ "surface": "slack", "channel": "#general" }]),
            "must not be posted",
            "must not be posted",
        ),
        1,
    )
    .await;
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, status::SKIPPED);
    assert_eq!(rows[0].selector, "#general");
    assert_eq!(rows[0].last_error.as_deref(), Some("unusable slack target"));
}

#[tokio::test]
async fn a_non_reviewed_status_enqueues_a_failure_notice_not_the_producers_bytes() {
    let h = harness("failed").await;
    bless_github(&h, "acme/widgets").await;
    set_and_route(
        &h,
        &envelope(
            "failed",
            json!([{ "surface": "github", "repo": "acme/widgets", "pr": 7 }]),
            "looks like a clean pass @octocat",
            "<!channel> ship it",
        ),
        1,
    )
    .await;
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(
        rows.len(),
        1,
        "the routing list still fires — never silence"
    );
    assert_eq!(rows[0].status, status::PENDING);
    let body = claim_body(&h).await.expect("enqueued");
    assert!(
        maidan_server::egress_body::comment_carries_result_marker(&body, h.thread_id),
        "a failure notice is still marked so a later review updates it"
    );
    assert!(
        body.contains(&maidan_server::result_delivery::failure_notice("failed")),
        "{body}"
    );
    assert!(!body.contains("clean pass") && !body.contains("@octocat"));
}

#[tokio::test]
async fn slack_receives_summary_never_rendered() {
    let h = harness("slack-summary").await;
    bless_slack(&h, "C0123ABCDEF").await;
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([{ "surface": "slack", "channel": "C0123ABCDEF" }]),
            "## Findings\n\nping @octocat",
            "3 findings",
        ),
        1,
    )
    .await;
    let body = claim_body(&h).await.expect("enqueued");
    assert!(body.contains("3 findings"), "{body}");
    assert!(
        !body.contains("Findings") && !body.contains("@octocat"),
        "slack must never receive rendered GFM: {body}"
    );
}

#[tokio::test]
async fn partial_delivery_skips_one_target_without_sinking_the_other() {
    let h = harness("partial").await;
    bless_github(&h, "acme/widgets").await;
    // Slack is aimed but not blessed.
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([
                { "surface": "github", "repo": "acme/widgets", "pr": 7 },
                { "surface": "slack", "channel": "C0123ABCDEF" }
            ]),
            "rendered body",
            "summary line",
        ),
        1,
    )
    .await;
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows.len(), 2, "one row per target");
    let gh = rows.iter().find(|r| r.surface == "github").unwrap();
    let sl = rows.iter().find(|r| r.surface == "slack").unwrap();
    assert_eq!(gh.status, status::PENDING);
    assert_eq!(sl.status, status::SKIPPED);
    let body = claim_body(&h).await.expect("github enqueued");
    assert!(body.contains("rendered body"));
    assert!(
        claim_body(&h).await.is_none(),
        "the skipped slack target must not have an outbox row"
    );
}

#[tokio::test]
async fn a_second_replica_of_the_same_event_does_not_reenqueue() {
    let h = harness("dedup").await;
    bless_github(&h, "acme/widgets").await;
    let result = envelope(
        "reviewed",
        json!([{ "surface": "github", "repo": "acme/widgets", "pr": 7 }]),
        "once",
        "once",
    );
    set_and_route(&h, &result, 11).await;
    // Same log_id, same stored result (same produced_at): the arm loses, and
    // the outbox unique key is a second backstop.
    notification_router::route_event(
        &h.state,
        11,
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
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(claim_body(&h).await.is_some());
    assert!(
        claim_body(&h).await.is_none(),
        "a replayed event must not leave a second outbox row"
    );
}

#[tokio::test]
async fn a_blessed_slack_target_on_a_server_with_no_slack_sender_fails_naming_the_token() {
    let h = harness_without_senders("no-slack").await;
    bless_slack(&h, "C0123ABCDEF").await;
    assert_unconfigured_target_fails(
        &h,
        json!({ "surface": "slack", "channel": "C0123ABCDEF", "thread_ts": "1700000000.000100" }),
        "slack",
        "Slack delivery is not configured on this server (MAIDAN_SLACK_BOT_TOKEN)",
    )
    .await;
}

#[tokio::test]
async fn a_blessed_linear_target_on_a_server_with_no_linear_sender_fails_naming_the_key() {
    let h = harness_without_senders("no-linear").await;
    bless_linear(&h).await;
    assert_unconfigured_target_fails(
        &h,
        json!({ "surface": "linear", "team": TEAM, "issue": "ENG-7" }),
        "linear",
        "Linear delivery is not configured on this server (MAIDAN_LINEAR_API_KEY)",
    )
    .await;
}

#[tokio::test]
async fn a_blessed_github_target_on_a_server_with_no_github_sender_fails_naming_the_setting() {
    let h = harness_without_senders("no-github").await;
    bless_github(&h, "acme/widgets").await;
    assert_unconfigured_target_fails(
        &h,
        json!({ "surface": "github", "repo": "acme/widgets", "pr": 7 }),
        "github",
        "GitHub delivery is not configured on this server (MAIDAN_GITHUB_TOKEN or a GitHub App)",
    )
    .await;
}

#[tokio::test]
async fn an_unblessed_target_on_a_server_with_no_sender_is_still_skipped_by_the_allowlist() {
    let h = harness_without_senders("no-slack-unblessed").await;
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([{ "surface": "slack", "channel": "C0123ABCDEF" }]),
            "rendered",
            "summary",
        ),
        1,
    )
    .await;
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, status::SKIPPED);
    assert_eq!(
        rows[0].last_error.as_deref(),
        Some("target not in the workspace egress allowlist")
    );
}

#[tokio::test]
async fn a_queued_result_whose_sender_is_gone_fails_when_the_worker_picks_it_up() {
    let mut h = harness("sender-gone").await;
    bless_slack(&h, "C0123ABCDEF").await;
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([{ "surface": "slack", "channel": "C0123ABCDEF" }]),
            "rendered",
            "summary",
        ),
        1,
    )
    .await;
    assert_eq!(
        h.store.list_result_deliveries(h.thread_id).await.unwrap()[0].status,
        status::PENDING
    );
    // The token is gone by the time the worker runs; GitHub's sender keeps
    // the worker running.
    h.state.slack_sender = None;

    let stats = egress_worker::sweep_once(&h.state).await;

    assert_eq!(stats.dead, 1, "{stats:?}");
    assert_eq!(
        stats.retried, 0,
        "a missing sender is not retried: {stats:?}"
    );
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows[0].status, status::FAILED);
    assert_eq!(
        rows[0].last_error.as_deref(),
        Some("Slack delivery is not configured on this server (MAIDAN_SLACK_BOT_TOKEN)")
    );
    assert_eq!(failed_events(&h).await.len(), 1);
    assert!(claim_body(&h).await.is_none(), "the queue entry is dead");
}

#[tokio::test]
async fn a_configured_surface_is_sent_while_another_surface_has_no_sender() {
    let mut h = harness_without_senders("slack-but-no-linear").await;
    h.state.attach_slack_sender(Arc::new(Posting));
    bless_slack(&h, "C0123ABCDEF").await;
    bless_linear(&h).await;
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([
                { "surface": "linear", "team": TEAM, "issue": "ENG-7" },
                { "surface": "slack", "channel": "C0123ABCDEF", "thread_ts": "1700000000.000100" }
            ]),
            "rendered",
            "summary",
        ),
        1,
    )
    .await;

    let stats = egress_worker::sweep_once(&h.state).await;

    assert_eq!(stats.sent, 1, "the slack result is sent: {stats:?}");
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    let slack = rows.iter().find(|r| r.surface == "slack").unwrap();
    let linear = rows.iter().find(|r| r.surface == "linear").unwrap();
    assert_eq!(slack.status, status::DELIVERED, "{slack:?}");
    assert_eq!(linear.status, status::FAILED, "{linear:?}");
    assert_eq!(
        linear.last_error.as_deref(),
        Some("Linear delivery is not configured on this server (MAIDAN_LINEAR_API_KEY)")
    );
}

#[tokio::test]
async fn a_send_that_will_be_retried_is_counted_on_the_delivery_row() {
    let h = harness("retried").await;
    bless_slack(&h, "C0123ABCDEF").await;
    set_and_route(
        &h,
        &envelope(
            "reviewed",
            json!([{ "surface": "slack", "channel": "C0123ABCDEF" }]),
            "rendered",
            "summary",
        ),
        1,
    )
    .await;

    let stats = egress_worker::sweep_once(&h.state).await;

    assert_eq!(stats.retried, 1, "{stats:?}");
    let rows = h.store.list_result_deliveries(h.thread_id).await.unwrap();
    assert_eq!(rows[0].status, status::PENDING, "still being retried");
    assert_eq!(rows[0].attempts, 1, "{rows:?}");
    assert_eq!(
        rows[0].last_error.as_deref(),
        Some("slack http error: these tests send nothing")
    );
}
