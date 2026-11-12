//! What a cancel cleans up, end to end against a fake GitHub: the cancel
//! queues the withdrawal of the pull request the change flow opened for the
//! task, and the egress worker (never the cancel itself) takes it out of the
//! merge queue and closes it with the reason. A pull request Maidan did not
//! open for the task is left alone, and a close that fails after the comment
//! is retried without a second comment. The context pack stops listing the
//! cancelled task's result as a decision, and the member whose claim a cancel
//! or someone else's close ended is notified.

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
use maidan_server::{
    egress_worker, github::GithubApiClient, notification_router, router, AppState,
    FederationRuntime,
};
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, EgressSurface, EgressTarget, Event, EventKind, MarkReadyBases, MemberId, MemberKind,
    MergeQueueLabels, NewApiToken, NewChangeRound, NewChannel, NewEgressTarget, NewMember,
    NewThread, NewWorkspace, ThreadId, ThreadState, WorkspaceId,
};
use serde_json::{json, Value};

const REPO: &str = "example-org/example-repo";
const LABEL: &str = "merge-queue";
const DELIVERED: &str = "1111111111111111111111111111111111111111";
const PRODUCER: &[&str] = &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE];

#[derive(Clone)]
struct FakePull {
    head: String,
    open: bool,
    labels: Vec<String>,
}

#[derive(Default)]
struct FakeGithub {
    pulls: HashMap<i64, FakePull>,
    label_deletes: Vec<(i64, String)>,
    comments: Vec<(i64, String)>,
    /// Comments written long before the test, listed ahead of `comments` and
    /// left out of a read with a later `since`, as GitHub leaves them out.
    history: Vec<(i64, String)>,
    /// The `since` of each comment read, `None` for a read without one.
    comment_reads: Vec<Option<String>>,
    closes: Vec<i64>,
    /// Closes still to fail with a 502, after which the pull request stays
    /// open.
    failing_closes: usize,
}

impl FakeGithub {
    fn writes_to(&self, number: i64) -> usize {
        self.label_deletes
            .iter()
            .filter(|(n, _)| *n == number)
            .count()
            + self.comments.iter().filter(|(n, _)| *n == number).count()
            + self.closes.iter().filter(|n| **n == number).count()
    }
}

type Shared = Arc<Mutex<FakeGithub>>;

async fn github(State(fake): State<Shared>, method: Method, uri: Uri, body: Bytes) -> Response {
    let path = uri.path().to_string();
    let mut fake = fake.lock().unwrap();
    let parts: Vec<&str> = path.split('/').collect();
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
        pull.labels.retain(|l| l != &label);
        let answer: Vec<Value> = pull.labels.iter().map(|l| json!({"name": l})).collect();
        return (StatusCode::OK, Json(Value::Array(answer))).into_response();
    }
    // /repos/{owner}/{name}/issues/{n}/comments, as the withdrawal reads
    // them for its own marker.
    if method == Method::GET && path.ends_with("/comments") {
        let number: i64 = parts[parts.len() - 2].parse().unwrap_or(-1);
        let query: HashMap<String, String> = uri
            .query()
            .map(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .into_owned()
                    .collect()
            })
            .unwrap_or_default();
        let since = query.get("since").cloned();
        fake.comment_reads.push(since.clone());
        // The history predates any `since` the withdrawal can send.
        let history = fake
            .history
            .iter()
            .filter(|_| since.is_none())
            .map(|(n, text)| (*n, text));
        let page: usize = query.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
        let per_page: usize = query
            .get("per_page")
            .and_then(|p| p.parse().ok())
            .unwrap_or(30);
        let listed: Vec<Value> = history
            .chain(fake.comments.iter().map(|(n, text)| (*n, text)))
            .filter(|(n, _)| *n == number)
            .enumerate()
            .skip((page - 1) * per_page)
            .take(per_page)
            .map(|(i, (_, text))| json!({"id": 1000 + i, "body": text}))
            .collect();
        return (StatusCode::OK, Json(Value::Array(listed))).into_response();
    }
    // /repos/{owner}/{name}/issues/{n}/comments
    if method == Method::POST && path.ends_with("/comments") {
        let number: i64 = parts[parts.len() - 2].parse().unwrap_or(-1);
        let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let text = payload["body"].as_str().unwrap_or_default().to_string();
        fake.comments.push((number, text.clone()));
        return (StatusCode::CREATED, Json(json!({"id": 501, "body": text}))).into_response();
    }
    // /repos/{owner}/{name}/pulls/{n}
    if path.contains("/pulls/") {
        let number: i64 = parts.last().and_then(|n| n.parse().ok()).unwrap_or(-1);
        if method == Method::PATCH {
            let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            if payload["state"] == "closed" && fake.failing_closes > 0 {
                fake.failing_closes -= 1;
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"message": "Bad Gateway"})),
                )
                    .into_response();
            }
            if payload["state"] == "closed" {
                fake.closes.push(number);
                if let Some(pull) = fake.pulls.get_mut(&number) {
                    pull.open = false;
                }
            }
            let open = fake.pulls.get(&number).is_some_and(|p| p.open);
            let state = if open { "open" } else { "closed" };
            return (
                StatusCode::OK,
                Json(json!({"number": number, "state": state})),
            )
                .into_response();
        }
        if method == Method::GET {
            return match fake.pulls.get(&number) {
                Some(p) => (
                    StatusCode::OK,
                    Json(json!({
                        "number": number,
                        "head": {"ref": p.head, "sha": DELIVERED, "repo": {"full_name": REPO}},
                        "base": {"ref": "dev", "repo": {"full_name": REPO}},
                        "draft": false,
                        "state": if p.open { "open" } else { "closed" },
                        "node_id": format!("PR_{number}"),
                        "labels": p.labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(),
                    })),
                )
                    .into_response(),
                None => {
                    (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))).into_response()
                }
            };
        }
    }
    (StatusCode::NOT_FOUND, Json(json!({}))).into_response()
}

fn queued(head: &str) -> FakePull {
    FakePull {
        head: head.into(),
        open: true,
        labels: vec![LABEL.into()],
    }
}

struct Harness {
    base: String,
    state: AppState,
    store: Arc<dyn Store>,
    pool: sqlx::SqlitePool,
    fake: Shared,
    ws: WorkspaceId,
    channel: ChannelId,
    producer: MemberId,
    producer_token: String,
}

impl Harness {
    async fn member(&self, handle: &str) -> MemberId {
        self.store
            .create_member(NewMember {
                workspace_id: self.ws,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap()
            .id
    }

    async fn task(&self, title: &str) -> ThreadId {
        self.store
            .create_thread(NewThread {
                channel_id: self.channel,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
                created_by: Some(self.producer),
            })
            .await
            .unwrap()
            .id
    }

    /// The change flow's round on `pull_number` for `thread`: Maidan opened
    /// that pull request for the task.
    async fn opened_for(&self, thread: ThreadId, pull_number: i64) {
        self.store
            .record_change_round(NewChangeRound {
                workspace_id: self.ws,
                thread_id: thread,
                repo: REPO.into(),
                pull_number,
                commit_sha: DELIVERED.into(),
                base_sha: "b".repeat(40),
                base_behind: None,
                diff_lines: 2,
                paths: vec!["src/lib.rs".into()],
                base_branch: Some("dev".into()),
                rebase: false,
                previous_head: None,
                linear_issue: None,
            })
            .await
            .unwrap();
    }

    /// `thread`'s change result delivered to `pull_number`.
    async fn delivered_by(&self, thread: ThreadId, pull_number: i64) {
        let target = EgressTarget::GithubBranch {
            repo: REPO.into(),
            branch: "feature/agent-x".into(),
        };
        let row = self
            .store
            .arm_result_delivery(thread, &target, Utc::now())
            .await
            .unwrap()
            .unwrap();
        self.store
            .mark_result_delivered(
                row.id,
                Some(&format!("{DELIVERED}#{pull_number}")),
                Utc::now(),
            )
            .await
            .unwrap();
    }

    async fn cancel(&self, thread: ThreadId, reason: &str) {
        let resp = reqwest::Client::new()
            .post(format!("{}/threads/{}/cancel", self.base, thread.0))
            .bearer_auth(&self.producer_token)
            .json(&json!({ "reason": reason }))
            .send()
            .await
            .unwrap();
        let status = resp.status();
        if status != StatusCode::OK {
            panic!(
                "cancel: {status}: {}",
                resp.text().await.unwrap_or_default()
            );
        }
    }
}

async fn spawn(pulls: &[(i64, FakePull)]) -> Harness {
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let fake: Shared = Arc::default();
    for (number, pull) in pulls {
        fake.lock().unwrap().pulls.insert(*number, pull.clone());
    }
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
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.attach_github_sender(Arc::new(
        GithubApiClient::with_base_url("test-token".into(), github_base).with_any_write_repo(),
    ));
    state.mark_ready_bases = MarkReadyBases::parse(&format!("{REPO}=dev")).unwrap();
    state.merge_queue_labels = MergeQueueLabels::parse(&format!("{REPO}={LABEL}")).unwrap();

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    store
        .allow_egress_target(NewEgressTarget {
            workspace_id: ws.id,
            surface: EgressSurface::GithubBranch,
            selector: format!("{REPO}@dev"),
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let producer = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "producer".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: producer.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: PRODUCER.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        state,
        store,
        pool,
        fake,
        ws: ws.id,
        channel: channel.id,
        producer: producer.id,
        producer_token: secret.as_str().to_string(),
    }
}

#[tokio::test]
async fn a_cancel_dequeues_and_closes_the_tasks_pull_request_through_the_outbox() {
    let h = spawn(&[(7, queued("feature/agent-x"))]).await;
    let thread = h.task("port the bridge").await;
    h.opened_for(thread, 7).await;
    h.delivered_by(thread, 7).await;

    h.cancel(thread, "the bridge is gone").await;
    assert_eq!(
        h.fake.lock().unwrap().writes_to(7),
        0,
        "the cancel itself writes nothing to GitHub"
    );

    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!(stats.sent, 1, "{stats:?}");
    {
        let fake = h.fake.lock().unwrap();
        assert_eq!(
            fake.label_deletes,
            vec![(7, LABEL.to_string())],
            "out of the merge queue"
        );
        assert_eq!(fake.comments.len(), 1, "{:?}", fake.comments);
        assert_eq!(fake.comments[0].0, 7);
        assert!(
            fake.comments[0].1.contains("the bridge is gone"),
            "the comment names the reason: {}",
            fake.comments[0].1
        );
        assert_eq!(fake.closes, vec![7], "the pull request is closed");
        let pull = &fake.pulls[&7];
        assert!(!pull.open);
        assert!(pull.labels.is_empty());
    }

    let again = egress_worker::sweep_once(&h.state).await;
    assert_eq!(again.sent, 0, "the withdrawal is sent once");
    assert_eq!(h.fake.lock().unwrap().closes, vec![7]);
}

#[tokio::test]
async fn a_close_that_fails_after_the_comment_is_retried_without_a_second_comment() {
    let h = spawn(&[(7, queued("feature/agent-x"))]).await;
    let thread = h.task("port the bridge").await;
    h.opened_for(thread, 7).await;
    h.delivered_by(thread, 7).await;
    h.fake.lock().unwrap().failing_closes = 1;

    h.cancel(thread, "the bridge is gone").await;
    let first = egress_worker::sweep_once(&h.state).await;
    assert_eq!(first.retried, 1, "the failed close is retried: {first:?}");
    {
        let fake = h.fake.lock().unwrap();
        assert_eq!(fake.comments.len(), 1, "GitHub took the comment");
        assert!(fake.closes.is_empty(), "but not the close");
        assert!(fake.pulls[&7].open);
    }

    // Due again now, rather than after the backoff.
    sqlx::query("UPDATE maidan_egress_outbox SET next_attempt_at = ? WHERE status = 'pending'")
        .bind(Utc::now().to_rfc3339())
        .execute(&h.pool)
        .await
        .unwrap();
    let retry = egress_worker::sweep_once(&h.state).await;
    assert_eq!(retry.sent, 1, "{retry:?}");
    let fake = h.fake.lock().unwrap();
    assert_eq!(fake.closes, vec![7], "the retry closes the pull request");
    assert!(!fake.pulls[&7].open);
    assert_eq!(
        fake.comments.len(),
        1,
        "exactly one comment for one withdrawal: {:?}",
        fake.comments
    );
    assert!(
        fake.comments[0].1.starts_with("<!-- maidan:withdraw:"),
        "the comment carries the marker the retry looked for: {}",
        fake.comments[0].1
    );
    assert!(fake.comments[0].1.contains("the bridge is gone"));
}

#[tokio::test]
async fn a_retry_finds_its_comment_behind_more_than_a_thousand_older_ones() {
    let h = spawn(&[(7, queued("feature/agent-x"))]).await;
    let thread = h.task("port the bridge").await;
    h.opened_for(thread, 7).await;
    h.delivered_by(thread, 7).await;
    {
        let mut fake = h.fake.lock().unwrap();
        fake.history = (0..1200).map(|i| (7, format!("review note {i}"))).collect();
        fake.failing_closes = 1;
    }

    h.cancel(thread, "the bridge is gone").await;
    let first = egress_worker::sweep_once(&h.state).await;
    assert_eq!(first.retried, 1, "the failed close is retried: {first:?}");
    assert_eq!(
        h.fake.lock().unwrap().comments.len(),
        1,
        "GitHub took the comment"
    );

    sqlx::query("UPDATE maidan_egress_outbox SET next_attempt_at = ? WHERE status = 'pending'")
        .bind(Utc::now().to_rfc3339())
        .execute(&h.pool)
        .await
        .unwrap();
    let retry = egress_worker::sweep_once(&h.state).await;
    assert_eq!(retry.sent, 1, "{retry:?}");
    let fake = h.fake.lock().unwrap();
    assert_eq!(fake.closes, vec![7], "the retry closes the pull request");
    // A scan from the first comment stops at 1,000, short of the marker, and
    // the retry would comment a second time.
    assert_eq!(
        fake.comments.len(),
        1,
        "one comment for one withdrawal, past 1,200 older ones: {:?}",
        fake.comments
    );
    let since = fake
        .comment_reads
        .last()
        .cloned()
        .flatten()
        .expect("the marker scan reads from when the withdrawal was queued");
    let since = chrono::DateTime::parse_from_rfc3339(&since)
        .unwrap()
        .with_timezone(&Utc);
    assert!(
        since <= Utc::now() && since > Utc::now() - chrono::Duration::hours(1),
        "the scan starts shortly before the withdrawal was queued: {since}"
    );
}

#[tokio::test]
async fn a_pull_request_maidan_did_not_open_for_the_task_is_left_alone() {
    let h = spawn(&[
        (7, queued("feature/agent-x")),
        (8, queued("feature/agent-session")),
    ])
    .await;
    let thread = h.task("half done").await;
    // A session's pull request tracked on the task: Maidan did not open it.
    h.store
        .register_change_pull(h.ws, thread, REPO, 8, None)
        .await
        .unwrap();
    // Maidan opened #7 for this task, but another task's result was
    // delivered to it since, so that task answers for it now.
    h.opened_for(thread, 7).await;
    let successor = h.task("the follow-up").await;
    h.delivered_by(successor, 7).await;

    h.cancel(thread, "superseded by the follow-up").await;
    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!(stats.sent, 0, "{stats:?}");
    assert_eq!(
        stats.refused, 1,
        "only #7 was queued, and refused: {stats:?}"
    );
    let fake = h.fake.lock().unwrap();
    assert_eq!(
        fake.writes_to(8),
        0,
        "the session's pull request is untouched"
    );
    assert_eq!(
        fake.writes_to(7),
        0,
        "the successor's pull request is untouched"
    );
    for number in [7, 8] {
        let pull = &fake.pulls[&number];
        assert!(pull.open, "#{number} is still open");
        assert_eq!(pull.labels, [LABEL], "#{number} is still queued");
    }
}

#[tokio::test]
async fn the_pack_omits_a_cancelled_result_but_keeps_a_landed_one() {
    let h = spawn(&[]).await;
    let worker = h.member("worker").await;
    let landed = h.task("landed decision").await;
    h.store
        .set_thread_result_claiming(landed, worker, &json!({"decision": "use postgres"}))
        .await
        .unwrap();
    for action in [
        maidan_fsm::ThreadAction::StartReview,
        maidan_fsm::ThreadAction::Close,
    ] {
        h.store
            .transition_thread(landed, worker, action)
            .await
            .unwrap();
    }
    let cancelled = h.task("cancelled decision").await;
    h.store
        .set_thread_result_claiming(cancelled, worker, &json!({"decision": "use sqlite"}))
        .await
        .unwrap();
    h.cancel(cancelled, "not wanted after all").await;
    assert_eq!(
        h.store.get_thread(cancelled).await.unwrap().state,
        ThreadState::Closed
    );

    let claimer = h.task("next task").await;
    let packed: Value = reqwest::Client::new()
        .get(format!("{}/threads/{}/context", h.base, claimer.0))
        .bearer_auth(&h.producer_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let titles: Vec<&str> = packed["accepted_decisions"]
        .as_array()
        .unwrap_or_else(|| panic!("no accepted_decisions: {packed}"))
        .iter()
        .filter_map(|d| d["title"].as_str())
        .collect();
    assert_eq!(titles, vec!["landed decision"]);
}

#[tokio::test]
async fn the_former_holder_is_notified_when_its_task_is_cancelled() {
    let h = spawn(&[]).await;
    let worker = h.member("worker").await;
    let watcher = h.member("watcher").await;
    h.store.follow_member(watcher, worker).await.unwrap();
    let thread = h.task("long build").await;
    let claimed = h
        .store
        .claim_next_thread(h.channel, worker, Some(600), None)
        .await
        .unwrap()
        .expect("the worker claims it");
    assert_eq!(claimed.id, thread);

    h.cancel(thread, "the build is not needed").await;
    // The bus consumer is not running here; route what the cancel logged.
    for stored in h
        .store
        .list_thread_events_through(thread, i64::MAX)
        .await
        .unwrap()
    {
        if stored.kind != EventKind::ThreadStateChanged {
            continue;
        }
        let event: Event = serde_json::from_value(stored.opened_payload().unwrap()).unwrap();
        notification_router::route_event(&h.state, stored.id, &event)
            .await
            .unwrap();
    }

    for member in [worker, watcher] {
        let notes = h.store.list_notifications(member, false, 10).await.unwrap();
        assert_eq!(notes.len(), 1, "{member:?}: {notes:?}");
        assert_eq!(notes[0].kind, EventKind::ThreadStateChanged);
        assert_eq!(notes[0].thread_id, Some(thread));
        assert_eq!(notes[0].actor_id, Some(h.producer));
    }
    assert!(
        h.store
            .list_notifications(h.producer, false, 10)
            .await
            .unwrap()
            .is_empty(),
        "the member who cancelled is not told what it did"
    );
}

#[tokio::test]
async fn the_former_holder_is_notified_when_someone_else_closes_its_task() {
    let h = spawn(&[]).await;
    let worker = h.member("worker").await;
    let watcher = h.member("watcher").await;
    h.store.follow_member(watcher, worker).await.unwrap();
    let thread = h.task("ship the parser").await;
    let claimed = h
        .store
        .claim_next_thread(h.channel, worker, Some(600), None)
        .await
        .unwrap()
        .expect("the worker claims it");
    assert_eq!(claimed.id, thread);

    // The worker sends it to review and still holds it; the producer closes.
    h.store
        .transition_thread_with_event(thread, worker, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();
    let (closed, stored) = h
        .store
        .transition_thread_with_event(thread, h.producer, maidan_fsm::ThreadAction::Close)
        .await
        .unwrap();
    assert_eq!(closed.thread.assignee_id, None, "the close ended the claim");
    let event: Event = serde_json::from_value(stored[0].opened_payload().unwrap()).unwrap();
    let Event::ThreadStateChanged {
        to_state,
        previous_assignee_id,
        ..
    } = &event
    else {
        panic!("the close's own event comes first: {event:?}");
    };
    assert_eq!(*to_state, ThreadState::Closed);
    assert_eq!(
        *previous_assignee_id,
        Some(worker),
        "the event names the holder"
    );
    notification_router::route_event(&h.state, stored[0].id, &event)
        .await
        .unwrap();
    for member in [worker, watcher] {
        let notes = h.store.list_notifications(member, false, 10).await.unwrap();
        assert_eq!(notes.len(), 1, "{member:?}: {notes:?}");
        assert_eq!(notes[0].kind, EventKind::ThreadStateChanged);
        assert_eq!(notes[0].thread_id, Some(thread));
        assert_eq!(notes[0].actor_id, Some(h.producer));
    }
    assert!(
        h.store
            .list_notifications(h.producer, false, 10)
            .await
            .unwrap()
            .is_empty(),
        "the member who closed it is not told what it did"
    );

    // A holder that closes its own task already knows: the event names nobody.
    let own = h.task("tidy the docs").await;
    let claimed = h
        .store
        .claim_next_thread(h.channel, worker, Some(600), None)
        .await
        .unwrap()
        .expect("the worker claims it");
    assert_eq!(claimed.id, own);
    h.store
        .transition_thread_with_event(own, worker, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();
    let (_, stored) = h
        .store
        .transition_thread_with_event(own, worker, maidan_fsm::ThreadAction::Close)
        .await
        .unwrap();
    let event: Event = serde_json::from_value(stored[0].opened_payload().unwrap()).unwrap();
    let Event::ThreadStateChanged {
        previous_assignee_id,
        ..
    } = &event
    else {
        panic!("the close's own event comes first: {event:?}");
    };
    assert_eq!(*previous_assignee_id, None);
}
