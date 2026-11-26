//! A review thread's result posts a guarded `pre-review` commit status beside
//! its GitHub comment, through the real result-delivery path and egress
//! worker, with the production `GithubApiClient` pointed at an in-memory fake
//! of GitHub that records every call.
//!
//! The status is posted only for a review thread whose reporter holds the
//! `pre-review` governance skill, only under the context
//! `pre-review`, only for a `PASS` (or `MERGE`) or `FIX-FIRST` verdict, and only at the
//! head the thread was briefed on; its description is built by Maidan from
//! the reporter's own handle and family. Anything refused posts nothing and
//! leaves a warning on the delivery row. The comment is delivered every time.
//!
//! A PASS never hides another reporter's FIX-FIRST: GitHub shows only the
//! newest status of a context, so a PASS reads the head's `pre-review`
//! statuses first and is held with a warning while the newest one the
//! reporter did not write is a failure. A FIX-FIRST always posts. A review
//! thread cancelled after its result was queued posts no status. Those rules
//! run on both backends; Postgres skips when Docker is unavailable.

// The fake GitHub is a plain axum server, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::State,
    http::{Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Json, Router,
};
use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{egress_worker, github::GithubApiClient, notification_router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, ChannelId, EgressSurface, Event, MemberId, MemberKind, NewAuditEvent, NewChannel,
    NewEgressTarget, NewMember, NewThread, NewWorkspace, ResultDelivery, TaskAttributes, TaskKind,
    ThreadId, WorkspaceId, PRE_REVIEW_SKILL, WAITER_RESULT_SCHEMA,
};
use serde_json::{json, Value};

const REPO: &str = "example/repo";
const PR: i64 = 42;
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER_HEAD: &str = "fedcba9876543210fedcba9876543210fedcba98";
const TOKEN: &str = "ghp_test_token_value";

// ---------------------------------------------------------------- fake GitHub

#[derive(Default)]
struct Fake {
    /// Every call as `(method, path, body)`.
    calls: Vec<(String, String, Value)>,
    next_comment: i64,
    /// The commit statuses posted, per commit, newest first: the order
    /// GitHub lists them in.
    statuses: HashMap<String, Vec<Value>>,
    /// Answer the status list with a 502, as GitHub does when it is down.
    statuses_unreadable: bool,
}

type Shared = Arc<Mutex<Fake>>;

async fn github(
    State(fake): State<Shared>,
    method: Method,
    uri: Uri,
    body: axum::body::Bytes,
) -> Response {
    let mut fake = fake.lock().unwrap();
    let path = urlencoding::decode(uri.path()).unwrap().into_owned();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    fake.calls.push((method.to_string(), path.clone(), body));
    let listed = path
        .strip_prefix(&format!("/repos/{REPO}/commits/"))
        .and_then(|rest| rest.strip_suffix("/statuses"));
    if let Some(sha) = listed.filter(|_| method == Method::GET) {
        if fake.statuses_unreadable {
            return (StatusCode::BAD_GATEWAY, Json(json!({"message": "down"}))).into_response();
        }
        let listed = fake.statuses.get(sha).cloned().unwrap_or_default();
        return (StatusCode::OK, Json(json!(listed))).into_response();
    }
    if method != Method::POST {
        return (StatusCode::OK, Json(json!([]))).into_response();
    }
    if let Some(sha) = path.strip_prefix(&format!("/repos/{REPO}/statuses/")) {
        let posted = fake
            .calls
            .last()
            .map(|call| call.2.clone())
            .unwrap_or_default();
        fake.statuses
            .entry(sha.to_string())
            .or_default()
            .insert(0, posted);
        return (StatusCode::CREATED, Json(json!({}))).into_response();
    }
    if path.ends_with("/comments") {
        fake.next_comment += 1;
        return (StatusCode::CREATED, Json(json!({"id": fake.next_comment}))).into_response();
    }
    (StatusCode::CREATED, Json(json!({}))).into_response()
}

async fn spawn_github(fake: Shared) -> String {
    let app = Router::new().fallback(any(github)).with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

// ---------------------------------------------------------------- harness

struct Harness {
    store: Arc<dyn Store>,
    state: AppState,
    fake: Shared,
    next_log_id: Mutex<i64>,
    /// Keeps the Postgres container alive for the test's length.
    _container: Option<testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>>,
}

/// One workspace with a reviewer member and a channel, blessed to deliver to
/// [`REPO`].
struct Tenant {
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    reviewer: MemberId,
}

/// A thread whose result is delivered as the comment, and maybe a status.
struct Review {
    thread_id: ThreadId,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    reviewer: MemberId,
}

async fn harness() -> Harness {
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
    harness_on(store, search, None).await
}

/// The same harness on a Postgres container, or `None` when no Docker daemon
/// answers.
async fn postgres_harness() -> Option<Harness> {
    use testcontainers::{runners::AsyncRunner, ImageExt};

    let container = match testcontainers_modules::postgres::Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    maidan_store::run_postgres_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(maidan_store::PostgresStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));
    Some(harness_on(store, search, Some(container)).await)
}

async fn harness_on(
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
    container: Option<testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>>,
) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    std::mem::forget(dir);
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    let fake: Shared = Arc::default();
    let base = spawn_github(fake.clone()).await;
    state.attach_github_sender(Arc::new(
        GithubApiClient::with_base_url(TOKEN.into(), base).with_any_write_repo(),
    ));
    Harness {
        store,
        state,
        fake,
        next_log_id: Mutex::new(0),
        _container: container,
    }
}

impl Harness {
    /// A tenant whose reviewer an operator granted the `pre-review` skill.
    async fn tenant(&self, name: &str, reviewer_handle: &str) -> Tenant {
        let tenant = self.tenant_without_skill(name, reviewer_handle).await;
        self.grant_pre_review(&tenant).await;
        tenant
    }

    /// Grant the reviewer `pre-review` the way `POST /members/{id}/skills`
    /// does for a governance skill: through the audited governance grant, by
    /// an operator of the reviewer's workspace.
    async fn grant_pre_review(&self, tenant: &Tenant) {
        let admin = self
            .store
            .create_member(NewMember {
                workspace_id: tenant.workspace_id,
                handle: format!("admin-{}", uuid::Uuid::now_v7().simple()),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        self.store
            .grant_governance_skill_audited(
                tenant.reviewer,
                PRE_REVIEW_SKILL,
                NewAuditEvent {
                    scope: AuditScope::Workspace(tenant.workspace_id),
                    actor_id: Some(admin.id),
                    action: "member_skill.grant_governance".into(),
                    target_kind: Some("member".into()),
                    target_id: Some(tenant.reviewer.0),
                    metadata: json!({ "skill": PRE_REVIEW_SKILL }),
                },
            )
            .await
            .unwrap();
    }

    async fn tenant_without_skill(&self, name: &str, reviewer_handle: &str) -> Tenant {
        let ws = self
            .store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let reviewer = self
            .store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: reviewer_handle.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = self
            .store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "reviews".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        self.store
            .allow_egress_target(NewEgressTarget {
                workspace_id: ws.id,
                surface: EgressSurface::Github,
                selector: REPO.into(),
                created_by: None,
            })
            .await
            .unwrap();
        Tenant {
            workspace_id: ws.id,
            channel_id: channel.id,
            reviewer: reviewer.id,
        }
    }

    /// A thread of `kind` whose description is `description`, claimed by the
    /// tenant's reviewer.
    async fn thread(&self, tenant: &Tenant, kind: TaskKind, description: Option<String>) -> Review {
        let thread = self
            .store
            .create_thread(NewThread {
                channel_id: tenant.channel_id,
                parent_thread_id: None,
                title: Some("Second-family review".into()),
                description,
                created_by: None,
            })
            .await
            .unwrap();
        self.store
            .set_thread_attributes(
                thread.id,
                TaskAttributes {
                    kind,
                    ..TaskAttributes::default()
                },
            )
            .await
            .unwrap();
        Review {
            thread_id: thread.id,
            workspace_id: tenant.workspace_id,
            channel_id: tenant.channel_id,
            reviewer: tenant.reviewer,
        }
    }

    async fn review(&self, tenant: &Tenant) -> Review {
        self.review_at(tenant, HEAD).await
    }

    async fn review_at(&self, tenant: &Tenant, head: &str) -> Review {
        // The round line keeps two reviews of one head from reading as a
        // duplicate task: the store refuses to open the same task twice.
        let round = uuid::Uuid::now_v7().simple();
        let description =
            format!("Pull request: {REPO}#{PR}\nHead to review: {head}\nRound: {round}");
        self.thread(tenant, TaskKind::Review, Some(description))
            .await
    }

    /// Claim the thread as its reviewer, report usage under `provider` and
    /// `model` when given, set `result`, and deliver it.
    async fn deliver(&self, review: &Review, usage: Option<(&str, &str)>, result: &Value) {
        self.queue(review, usage, result).await;
        let _ = egress_worker::sweep_once(&self.state).await;
    }

    /// [`Self::deliver`] up to the egress worker's sweep: the result is
    /// stored and its delivery queued, and nothing is sent yet.
    async fn queue(&self, review: &Review, usage: Option<(&str, &str)>, result: &Value) {
        let claim = self
            .store
            .claim_thread(review.thread_id, review.reviewer)
            .await
            .unwrap();
        let lease = claim.thread.claim_lease_id.unwrap();
        if let Some((provider, model)) = usage {
            self.store
                .report_accounted_usage(&maidan_types::NewUsageLedgerEntry {
                    usage_report_id: uuid::Uuid::now_v7(),
                    thread_id: review.thread_id,
                    reporter: review.reviewer,
                    claim_lease_id: lease,
                    model: model.into(),
                    tokens: maidan_types::TokenUsage {
                        input: 1_000,
                        ..Default::default()
                    },
                    usd_micros: 10,
                    price_snapshot: maidan_types::PriceSnapshot {
                        input_usd_micros_per_million: 10_000,
                        ..Default::default()
                    },
                    turns: 1,
                    evidence: maidan_types::UsageEvidence {
                        provider: Some(provider.into()),
                        ..Default::default()
                    },
                })
                .await
                .unwrap();
        }
        self.store
            .set_thread_result(review.thread_id, review.reviewer, lease, result)
            .await
            .unwrap();
        let log_id = {
            let mut next = self.next_log_id.lock().unwrap();
            *next += 1;
            *next
        };
        notification_router::route_event(
            &self.state,
            log_id,
            &Event::ThreadResultSet {
                occurred_at: Utc::now(),
                workspace_id: review.workspace_id,
                channel_id: review.channel_id,
                thread_id: review.thread_id,
                produced_by: review.reviewer,
            },
        )
        .await
        .unwrap();
    }

    /// Another reviewer in the tenant's workspace, granted `pre-review`.
    async fn add_reviewer(&self, tenant: &Tenant, handle: &str) -> Tenant {
        let reviewer = self
            .store
            .create_member(NewMember {
                workspace_id: tenant.workspace_id,
                handle: handle.into(),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let other = Tenant {
            workspace_id: tenant.workspace_id,
            channel_id: tenant.channel_id,
            reviewer: reviewer.id,
        };
        self.grant_pre_review(&other).await;
        other
    }

    /// The commit statuses posted, as `(path, body)`.
    fn statuses(&self) -> Vec<(String, Value)> {
        self.fake
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, path, _)| method == "POST" && path.contains("/statuses/"))
            .map(|(_, path, body)| (path.clone(), body.clone()))
            .collect()
    }

    /// The statuses GitHub lists on `sha`, newest first.
    fn listed(&self, sha: &str) -> Vec<Value> {
        self.fake
            .lock()
            .unwrap()
            .statuses
            .get(sha)
            .cloned()
            .unwrap_or_default()
    }

    /// A status another system posted on `sha`, newer than any listed.
    fn seed_status(&self, sha: &str, context: &str, state: &str, description: &str) {
        self.fake
            .lock()
            .unwrap()
            .statuses
            .entry(sha.to_string())
            .or_default()
            .insert(
                0,
                json!({"context": context, "state": state, "description": description}),
            );
    }

    /// How many times the status list was read.
    fn status_reads(&self) -> usize {
        self.fake
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, path, _)| method == "GET" && path.ends_with("/statuses"))
            .count()
    }

    /// The bodies of the result comments written on [`PR`]: the first round
    /// posts one, and a later round edits it in place.
    fn comments(&self) -> Vec<String> {
        let posted = format!("/repos/{REPO}/issues/{PR}/comments");
        let edited = format!("/repos/{REPO}/issues/comments/");
        self.fake
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, p, _)| {
                (method == "POST" && *p == posted) || (method == "PATCH" && p.starts_with(&edited))
            })
            .map(|(_, _, body)| body["body"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    async fn github_row(&self, review: &Review) -> ResultDelivery {
        let rows = self
            .store
            .list_result_deliveries(review.thread_id)
            .await
            .unwrap();
        let github: Vec<_> = rows.into_iter().filter(|r| r.surface == "github").collect();
        assert_eq!(github.len(), 1, "one github delivery row: {github:?}");
        github.into_iter().next().unwrap()
    }
}

/// A review agent's result, as it sets one.
fn review_result(verdict: &str, context: &str, head: &str, rendered: &str) -> Value {
    json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": "deepseek.redteam/1",
        "status": "reviewed",
        "verdict": verdict,
        "head_sha": head,
        "pr": format!("{REPO}#{PR}"),
        "rendered": rendered,
        "findings": [],
        "deliver_to": [{"surface": "github", "repo": REPO, "pr": PR}],
        // What a model might write: the state and description are ignored.
        "check": {"context": context, "state": "success", "description": "LGTM ship it"},
    })
}

fn assert_comment_delivered(h: &Harness, rendered: &str) {
    let comments = h.comments();
    assert!(
        comments.iter().any(|body| body.contains(rendered)),
        "the comment carrying {rendered:?} was delivered: {comments:?}"
    );
}

// ---------------------------------------------------------------- tests

#[tokio::test]
async fn a_pass_posts_success_at_the_briefed_head_named_by_the_reporters_own_usage_family() {
    let h = harness().await;
    let tenant = h.tenant("alpha", "reviewer-a").await;
    let review = h.review(&tenant).await;
    // The result_kind says deepseek; the reporter's own usage says openai,
    // and the usage is what names who reviewed.
    h.deliver(
        &review,
        Some(("openai", "gpt-5")),
        // A red-team review says MERGE for a pass; the status says PASS.
        &review_result("MERGE", "pre-review", HEAD, "Review: PASS"),
    )
    .await;

    let statuses = h.statuses();
    assert_eq!(statuses.len(), 1, "{statuses:?}");
    let (path, body) = &statuses[0];
    assert_eq!(path, &format!("/repos/{REPO}/statuses/{HEAD}"));
    assert_eq!(body["context"], "pre-review");
    assert_eq!(body["state"], "success");
    assert_eq!(
        body["description"],
        "PASS at 012345678 by openai as reviewer-a"
    );
    assert!(body.get("target_url").is_none(), "{body}");
    assert_comment_delivered(&h, "Review: PASS");
    let row = h.github_row(&review).await;
    assert_eq!(row.status, maidan_types::status::DELIVERED);
    assert!(row.delivery_warnings.is_empty(), "{row:?}");
}

#[tokio::test]
async fn a_fix_first_posts_failure_with_the_family_its_result_kind_names_when_no_usage_does() {
    let h = harness().await;
    let tenant = h.tenant("alpha", "reviewer-a").await;
    let review = h.review(&tenant).await;
    // The model asked for `success`; the verdict decides.
    h.deliver(
        &review,
        None,
        &review_result("FIX-FIRST", "pre-review", HEAD, "Review: FIX-FIRST"),
    )
    .await;

    let statuses = h.statuses();
    assert_eq!(statuses.len(), 1, "{statuses:?}");
    let (path, body) = &statuses[0];
    assert_eq!(path, &format!("/repos/{REPO}/statuses/{HEAD}"));
    assert_eq!(body["context"], "pre-review");
    assert_eq!(body["state"], "failure");
    assert_eq!(
        body["description"],
        "FIX-FIRST at 012345678 by deepseek as reviewer-a"
    );
    assert_comment_delivered(&h, "Review: FIX-FIRST");
    assert!(h.github_row(&review).await.delivery_warnings.is_empty());
}

#[tokio::test]
async fn a_review_naming_another_context_posts_nothing_and_warns_naming_it() {
    let h = harness().await;
    let tenant = h.tenant("alpha", "reviewer-a").await;
    let review = h.review(&tenant).await;
    h.deliver(
        &review,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "stamp", HEAD, "Review: stamp attempt"),
    )
    .await;

    assert!(h.statuses().is_empty(), "{:?}", h.statuses());
    assert_comment_delivered(&h, "Review: stamp attempt");
    let row = h.github_row(&review).await;
    assert_eq!(row.status, maidan_types::status::DELIVERED);
    assert_eq!(
        row.delivery_warnings,
        vec!["pre_review_context_refused: stamp".to_string()]
    );
}

#[tokio::test]
async fn a_review_of_another_head_than_the_briefed_one_posts_nothing_and_warns_naming_both() {
    let h = harness().await;
    let tenant = h.tenant("alpha", "reviewer-a").await;
    let review = h.review(&tenant).await;
    h.deliver(
        &review,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", OTHER_HEAD, "Review: moved head"),
    )
    .await;

    assert!(h.statuses().is_empty(), "{:?}", h.statuses());
    assert_comment_delivered(&h, "Review: moved head");
    let row = h.github_row(&review).await;
    assert_eq!(row.delivery_warnings.len(), 1, "{row:?}");
    let warning = &row.delivery_warnings[0];
    assert!(
        warning.starts_with("pre_review_head_refused:")
            && warning.contains(HEAD)
            && warning.contains(OTHER_HEAD),
        "{warning}"
    );
}

#[tokio::test]
async fn a_review_thread_without_a_briefed_head_posts_nothing_and_warns() {
    let h = harness().await;
    let tenant = h.tenant("alpha", "reviewer-a").await;
    let review = h
        .thread(
            &tenant,
            TaskKind::Review,
            Some(format!("Pull request: {REPO}#{PR}")),
        )
        .await;
    h.deliver(
        &review,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: unbriefed"),
    )
    .await;

    assert!(h.statuses().is_empty(), "{:?}", h.statuses());
    assert_comment_delivered(&h, "Review: unbriefed");
    let row = h.github_row(&review).await;
    assert_eq!(row.delivery_warnings.len(), 1, "{row:?}");
    assert!(
        row.delivery_warnings[0].contains("no head") && row.delivery_warnings[0].contains(HEAD),
        "{row:?}"
    );
}

#[tokio::test]
async fn a_result_on_a_thread_that_is_not_a_review_posts_no_status() {
    let h = harness().await;
    let tenant = h.tenant("alpha", "reviewer-a").await;
    let description = format!("Head to review: {HEAD}");
    let change = h.thread(&tenant, TaskKind::Change, Some(description)).await;
    h.deliver(
        &change,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: on a change thread"),
    )
    .await;

    assert!(h.statuses().is_empty(), "{:?}", h.statuses());
    assert_comment_delivered(&h, "Review: on a change thread");
    let row = h.github_row(&change).await;
    assert_eq!(row.status, maidan_types::status::DELIVERED);
    assert!(row.delivery_warnings.is_empty(), "{row:?}");
}

#[tokio::test]
async fn each_tenants_status_names_its_own_reporter_and_family() {
    let h = harness().await;
    let alpha = h.tenant("alpha", "reviewer-a").await;
    let beta = h.tenant("beta", "reviewer-b").await;
    let in_alpha = h.review(&alpha).await;
    let in_beta = h.review(&beta).await;
    h.deliver(
        &in_alpha,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: alpha"),
    )
    .await;
    h.deliver(
        &in_beta,
        Some(("xai", "grok-4")),
        &review_result("FIX-FIRST", "pre-review", HEAD, "Review: beta"),
    )
    .await;

    let mut descriptions: Vec<(String, String)> = h
        .statuses()
        .into_iter()
        .map(|(_, body)| {
            (
                body["state"].as_str().unwrap().to_string(),
                body["description"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    descriptions.sort();
    assert_eq!(
        descriptions,
        vec![
            (
                "failure".to_string(),
                "FIX-FIRST at 012345678 by xai as reviewer-b".to_string()
            ),
            (
                "success".to_string(),
                "PASS at 012345678 by deepseek as reviewer-a".to_string()
            ),
        ]
    );
    assert_comment_delivered(&h, "Review: alpha");
    assert_comment_delivered(&h, "Review: beta");
}

#[tokio::test]
async fn a_reviewer_without_the_pre_review_skill_posts_nothing_and_warns_naming_it() {
    let h = harness().await;
    let tenant = h.tenant_without_skill("alpha", "reviewer-a").await;
    let review = h.review(&tenant).await;
    h.deliver(
        &review,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: ungranted"),
    )
    .await;

    assert!(h.statuses().is_empty(), "{:?}", h.statuses());
    assert_comment_delivered(&h, "Review: ungranted");
    let row = h.github_row(&review).await;
    assert_eq!(row.status, maidan_types::status::DELIVERED);
    assert_eq!(
        row.delivery_warnings,
        vec!["pre_review_skill_missing: reviewer-a does not hold the pre-review skill".to_string()]
    );
}

#[tokio::test]
async fn a_revoked_pre_review_skill_stops_the_next_status() {
    let h = harness().await;
    let tenant = h.tenant("alpha", "reviewer-a").await;
    let review = h.review(&tenant).await;
    h.deliver(
        &review,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: first round"),
    )
    .await;
    assert_eq!(h.statuses().len(), 1, "{:?}", h.statuses());

    // Held is read at delivery, not at claim: an operator's revoke stops the
    // next round's status.
    assert!(h
        .store
        .remove_member_skill(tenant.reviewer, PRE_REVIEW_SKILL)
        .await
        .unwrap());
    h.store.unassign_thread(review.thread_id).await.unwrap();
    // A newer revision, so the delivery row re-arms.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    h.deliver(
        &review,
        Some(("deepseek", "deepseek-chat")),
        &review_result("FIX-FIRST", "pre-review", HEAD, "Review: second round"),
    )
    .await;

    assert_eq!(
        h.statuses().len(),
        1,
        "no second status: {:?}",
        h.statuses()
    );
    assert_comment_delivered(&h, "Review: second round");
    let row = h.github_row(&review).await;
    assert_eq!(row.delivery_warnings.len(), 1, "{row:?}");
    assert!(
        row.delivery_warnings[0].starts_with("pre_review_skill_missing:"),
        "{row:?}"
    );
}

#[tokio::test]
async fn a_pre_review_skill_held_in_another_workspace_does_not_count() {
    let h = harness().await;
    let alpha = h.tenant_without_skill("alpha", "reviewer-a").await;
    // Another workspace's operator grants its own reviewer; alpha's reviewer
    // stays without it.
    let beta = h.tenant("beta", "reviewer-a").await;
    assert_ne!(alpha.reviewer, beta.reviewer);
    let review = h.review(&alpha).await;
    h.deliver(
        &review,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: alpha ungranted"),
    )
    .await;

    assert!(h.statuses().is_empty(), "{:?}", h.statuses());
    let row = h.github_row(&review).await;
    assert!(
        row.delivery_warnings[0].starts_with("pre_review_skill_missing:"),
        "{row:?}"
    );
}

// ------------------------------------------- a PASS never hides a FIX-FIRST

async fn a_pass_after_another_reporters_fix_first_is_held(h: &Harness) {
    let alpha = h.tenant("alpha", "reviewer-a").await;
    let beta = h.add_reviewer(&alpha, "reviewer-b").await;
    let first = h.review(&alpha).await;
    let second = h.review(&beta).await;
    h.deliver(
        &first,
        Some(("deepseek", "deepseek-chat")),
        &review_result("FIX-FIRST", "pre-review", HEAD, "Review: first"),
    )
    .await;
    h.deliver(
        &second,
        Some(("xai", "grok-4")),
        &review_result("MERGE", "pre-review", HEAD, "Review: second"),
    )
    .await;

    // Only the FIX-FIRST was posted, and it is still the newest status.
    let statuses = h.statuses();
    assert_eq!(statuses.len(), 1, "{statuses:?}");
    assert_eq!(statuses[0].1["state"], "failure");
    let listed = h.listed(HEAD);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["state"], "failure");
    // The held PASS's comment is delivered, and its row names why.
    assert_comment_delivered(h, "Review: second");
    let row = h.github_row(&second).await;
    assert_eq!(row.status, maidan_types::status::DELIVERED);
    assert_eq!(row.delivery_warnings.len(), 1, "{row:?}");
    let warning = &row.delivery_warnings[0];
    assert!(
        warning.starts_with("pre_review_fix_first_stands:")
            && warning.contains("FIX-FIRST at 012345678 by deepseek as reviewer-a"),
        "{warning}"
    );
    assert!(h.github_row(&first).await.delivery_warnings.is_empty());
}

#[tokio::test]
async fn a_pass_after_another_reporters_fix_first_is_held_with_a_warning_on_sqlite() {
    a_pass_after_another_reporters_fix_first_is_held(&harness().await).await;
}

#[tokio::test]
async fn a_pass_after_another_reporters_fix_first_is_held_with_a_warning_on_postgres() {
    let Some(h) = postgres_harness().await else {
        return;
    };
    a_pass_after_another_reporters_fix_first_is_held(&h).await;
}

async fn a_fix_first_after_another_reporters_pass_posts(h: &Harness) {
    let alpha = h.tenant("alpha", "reviewer-a").await;
    let beta = h.add_reviewer(&alpha, "reviewer-b").await;
    let first = h.review(&alpha).await;
    let second = h.review(&beta).await;
    h.deliver(
        &first,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: first"),
    )
    .await;
    h.deliver(
        &second,
        Some(("xai", "grok-4")),
        &review_result("FIX-FIRST", "pre-review", HEAD, "Review: second"),
    )
    .await;

    let states: Vec<_> = h.listed(HEAD).iter().map(|s| s["state"].clone()).collect();
    assert_eq!(states, vec![json!("failure"), json!("success")]);
    assert!(h.github_row(&first).await.delivery_warnings.is_empty());
    assert!(h.github_row(&second).await.delivery_warnings.is_empty());
    // Only the PASS reads the list: a FIX-FIRST is never held.
    assert_eq!(h.status_reads(), 1);
}

#[tokio::test]
async fn a_fix_first_after_another_reporters_pass_posts_on_sqlite() {
    a_fix_first_after_another_reporters_pass_posts(&harness().await).await;
}

#[tokio::test]
async fn a_fix_first_after_another_reporters_pass_posts_on_postgres() {
    let Some(h) = postgres_harness().await else {
        return;
    };
    a_fix_first_after_another_reporters_pass_posts(&h).await;
}

async fn a_reporter_replaces_its_own_fix_first_with_a_pass(h: &Harness) {
    let tenant = h.tenant("alpha", "reviewer-a").await;
    let first = h.review(&tenant).await;
    let again = h.review(&tenant).await;
    h.deliver(
        &first,
        Some(("deepseek", "deepseek-chat")),
        &review_result("FIX-FIRST", "pre-review", HEAD, "Review: first round"),
    )
    .await;
    h.deliver(
        &again,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: second round"),
    )
    .await;

    let states: Vec<_> = h.listed(HEAD).iter().map(|s| s["state"].clone()).collect();
    assert_eq!(states, vec![json!("success"), json!("failure")]);
    assert!(h.github_row(&again).await.delivery_warnings.is_empty());
}

#[tokio::test]
async fn a_reporter_replaces_its_own_fix_first_with_a_pass_on_sqlite() {
    a_reporter_replaces_its_own_fix_first_with_a_pass(&harness().await).await;
}

#[tokio::test]
async fn a_reporter_replaces_its_own_fix_first_with_a_pass_on_postgres() {
    let Some(h) = postgres_harness().await else {
        return;
    };
    a_reporter_replaces_its_own_fix_first_with_a_pass(&h).await;
}

async fn a_failure_no_review_wrote_holds_a_pass_but_another_context_does_not(h: &Harness) {
    let tenant = h.tenant("alpha", "reviewer-a").await;
    // A person or another system left a failing `pre-review` on one head and
    // a failing `stamp` on another.
    h.seed_status(HEAD, "pre-review", "failure", "needs work");
    h.seed_status(OTHER_HEAD, "stamp", "failure", "stamp refused");
    let held = h.review_at(&tenant, HEAD).await;
    let posted = h.review_at(&tenant, OTHER_HEAD).await;
    h.deliver(
        &held,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: held"),
    )
    .await;
    h.deliver(
        &posted,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", OTHER_HEAD, "Review: posted"),
    )
    .await;

    let statuses = h.statuses();
    assert_eq!(statuses.len(), 1, "{statuses:?}");
    assert_eq!(
        statuses[0].0,
        format!("/repos/{REPO}/statuses/{OTHER_HEAD}")
    );
    let row = h.github_row(&held).await;
    assert_eq!(row.delivery_warnings.len(), 1, "{row:?}");
    assert!(
        row.delivery_warnings[0].starts_with("pre_review_fix_first_stands:")
            && row.delivery_warnings[0].contains("needs work"),
        "{row:?}"
    );
    assert!(h.github_row(&posted).await.delivery_warnings.is_empty());
}

#[tokio::test]
async fn a_failure_no_review_wrote_holds_a_pass_but_another_context_does_not_on_sqlite() {
    a_failure_no_review_wrote_holds_a_pass_but_another_context_does_not(&harness().await).await;
}

#[tokio::test]
async fn a_failure_no_review_wrote_holds_a_pass_but_another_context_does_not_on_postgres() {
    let Some(h) = postgres_harness().await else {
        return;
    };
    a_failure_no_review_wrote_holds_a_pass_but_another_context_does_not(&h).await;
}

async fn a_pass_is_held_when_the_statuses_cannot_be_read_and_a_fix_first_still_posts(h: &Harness) {
    let alpha = h.tenant("alpha", "reviewer-a").await;
    let beta = h.add_reviewer(&alpha, "reviewer-b").await;
    h.fake.lock().unwrap().statuses_unreadable = true;
    let pass = h.review(&alpha).await;
    let fix = h.review_at(&beta, OTHER_HEAD).await;
    h.deliver(
        &pass,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: unread"),
    )
    .await;
    h.deliver(
        &fix,
        Some(("xai", "grok-4")),
        &review_result("FIX-FIRST", "pre-review", OTHER_HEAD, "Review: fix"),
    )
    .await;

    let statuses = h.statuses();
    assert_eq!(statuses.len(), 1, "{statuses:?}");
    assert_eq!(statuses[0].1["state"], "failure");
    let row = h.github_row(&pass).await;
    assert_eq!(row.status, maidan_types::status::DELIVERED);
    assert_eq!(row.delivery_warnings.len(), 1, "{row:?}");
    assert!(
        row.delivery_warnings[0].starts_with("pre_review_statuses_unread:"),
        "{row:?}"
    );
    assert!(h.github_row(&fix).await.delivery_warnings.is_empty());
    assert_eq!(h.status_reads(), 1);
}

#[tokio::test]
async fn a_pass_is_held_when_the_statuses_cannot_be_read_and_a_fix_first_still_posts_on_sqlite() {
    a_pass_is_held_when_the_statuses_cannot_be_read_and_a_fix_first_still_posts(&harness().await)
        .await;
}

#[tokio::test]
async fn a_pass_is_held_when_the_statuses_cannot_be_read_and_a_fix_first_still_posts_on_postgres() {
    let Some(h) = postgres_harness().await else {
        return;
    };
    a_pass_is_held_when_the_statuses_cannot_be_read_and_a_fix_first_still_posts(&h).await;
}

async fn a_status_queued_for_a_thread_that_was_cancelled_is_not_posted(h: &Harness) {
    let alpha = h.tenant("alpha", "reviewer-a").await;
    let beta = h.add_reviewer(&alpha, "reviewer-b").await;
    let pass = h.review(&alpha).await;
    let fix = h.review(&beta).await;
    h.queue(
        &pass,
        Some(("deepseek", "deepseek-chat")),
        &review_result("PASS", "pre-review", HEAD, "Review: cancelled pass"),
    )
    .await;
    h.queue(
        &fix,
        Some(("xai", "grok-4")),
        &review_result("FIX-FIRST", "pre-review", HEAD, "Review: cancelled fix"),
    )
    .await;
    // Cancelled while the deliveries wait in the queue.
    for review in [&pass, &fix] {
        let outcome = h
            .store
            .cancel_thread(
                review.thread_id,
                review.reviewer,
                "no longer needed",
                maidan_types::CancelAuthority::Transition,
            )
            .await
            .unwrap();
        assert!(
            matches!(outcome, maidan_types::CancelOutcome::Cancelled { .. }),
            "{outcome:?}"
        );
    }
    let _ = egress_worker::sweep_once(&h.state).await;

    assert!(h.statuses().is_empty(), "{:?}", h.statuses());
    assert!(h.listed(HEAD).is_empty());
    assert_eq!(h.status_reads(), 0, "a cancelled review reads nothing");
    for review in [&pass, &fix] {
        let row = h.github_row(review).await;
        assert_eq!(
            row.delivery_warnings,
            vec![
                "pre_review_thread_ended: the review was cancelled after its result was queued"
                    .to_string()
            ],
            "{row:?}"
        );
    }
}

#[tokio::test]
async fn a_status_queued_for_a_thread_that_was_cancelled_is_not_posted_on_sqlite() {
    a_status_queued_for_a_thread_that_was_cancelled_is_not_posted(&harness().await).await;
}

#[tokio::test]
async fn a_status_queued_for_a_thread_that_was_cancelled_is_not_posted_on_postgres() {
    let Some(h) = postgres_harness().await else {
        return;
    };
    a_status_queued_for_a_thread_that_was_cancelled_is_not_posted(&h).await;
}
