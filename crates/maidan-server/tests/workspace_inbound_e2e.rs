//! A workspace's own inbound URLs. A delivery to
//! `/workspaces/{wid}/connections/{surface}/...` is verified with the secret
//! that workspace stored and reaches that workspace's links and watches alone,
//! even where another workspace linked the same Slack channel or GitHub
//! issue. The server's own route reaches only the workspaces its credential
//! serves: opened by the operator, with no connection of their own.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::github::{route_message_to_github, GithubConfig, GithubError, GithubSender};
use maidan_server::slack::{slack_signature, SlackConfig};
use maidan_server::test_support::connect;
use maidan_server::webhooks::sign_payload;
use maidan_server::{egress_worker, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use reqwest::StatusCode;
use serde_json::{json, Value};

const INSTANCE_GITHUB_SECRET: &str = "instance-github-secret-0123456789";
const INSTANCE_SLACK_SECRET: &str = "instance-slack-secret-0123456789";
const ADA_SECRET: &str = "ada-inbound-secret-0123456789";
const BO_SECRET: &str = "bo-inbound-secret-0123456789";

/// Posts comments through itself and records the issues it posted to.
#[derive(Default)]
struct MockGithub {
    sent: Mutex<Vec<i64>>,
}

#[async_trait::async_trait]
impl GithubSender for MockGithub {
    async fn post_comment(
        &self,
        repo: &str,
        issue_number: i64,
        _text: &str,
    ) -> Result<Option<ExternalRef>, GithubError> {
        self.sent.lock().unwrap().push(issue_number);
        // The comment's id is made up from the issue, so a test can send the
        // webhook that echoes it.
        Ok(Some(ExternalRef::Github {
            repo: repo.into(),
            comment_id: 9000 + issue_number,
        }))
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

    async fn create_check_run(&self, _: &str, _: &GithubCheckRun) -> Result<(), GithubError> {
        unreachable!("the projector egress never creates a check run")
    }

    async fn create_review(
        &self,
        _: &str,
        _: i64,
        _: &str,
        _: &[GithubReviewComment],
    ) -> Result<(), GithubError> {
        unreachable!("the projector egress never creates a review")
    }
}

struct Harness {
    state: AppState,
    store: Arc<dyn Store>,
    base: String,
    http: reqwest::Client,
}

async fn harness(instance_inbound: bool) -> Harness {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::for_tests(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(16)),
        search,
    );
    std::mem::forget(dir);
    state.federation = FederationRuntime::new(true, Some(Arc::new([9u8; 32])));
    if instance_inbound {
        state.attach_github(Arc::new(GithubConfig {
            webhook_secret: INSTANCE_GITHUB_SECRET.into(),
            api_token: None,
            write_repos: Vec::new(),
        }));
        state.attach_slack(Arc::new(SlackConfig {
            signing_secret: INSTANCE_SLACK_SECRET.into(),
            bot_token: None,
        }));
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Harness {
        state,
        store,
        base: format!("http://{addr}"),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
    }
}

/// A workspace, its admin and a thread of its own.
struct Room {
    workspace: WorkspaceId,
    member: MemberId,
    channel: ChannelId,
    thread: ThreadId,
}

impl Harness {
    async fn signed_up(&self, who: &str) -> Room {
        let outcome = self
            .store
            .complete_oidc_signup(
                NewOidcSignup {
                    workspace_name: format!("{who}'s workspace"),
                    handle: who.into(),
                    issuer: "https://idp.example".into(),
                    subject: format!("sub-{who}"),
                    email: format!("{who}@example.com"),
                    invite_entry_id: None,
                    session_expires_at: chrono::Utc::now() + Duration::from_secs(3600),
                    open: false,
                },
                Box::new(|core| NewAuditEvent {
                    scope: AuditScope::Workspace(core.workspace.id),
                    actor_id: Some(core.member.id),
                    action: "workspace.signup".into(),
                    target_kind: Some("workspace".into()),
                    target_id: Some(core.workspace.id.0),
                    metadata: json!({}),
                }),
                Box::new(|session| NewAuditEvent {
                    scope: AuditScope::Workspace(session.workspace_id),
                    actor_id: Some(session.member_id),
                    action: "session.create".into(),
                    target_kind: Some("member".into()),
                    target_id: Some(session.member_id.0),
                    metadata: json!({}),
                }),
            )
            .await
            .unwrap();
        self.room(outcome.workspace.id, outcome.member.id).await
    }

    /// A workspace the operator opened, with an admin.
    async fn operator_workspace(&self, name: &str) -> Room {
        let ws = self
            .store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let member = self
            .store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "admin".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        self.room(ws.id, member.id).await
    }

    async fn room(&self, workspace: WorkspaceId, member: MemberId) -> Room {
        let channel = self
            .store
            .create_channel(NewChannel {
                workspace_id: workspace,
                name: "eng".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = self
            .store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: None,
                description: None,
                created_by: Some(member),
            })
            .await
            .unwrap();
        Room {
            workspace,
            member,
            channel: channel.id,
            thread: thread.id,
        }
    }

    async fn connect_both(&self, room: &Room, secret: &str) {
        for surface in [ConnectionSurface::Github, ConnectionSurface::Slack] {
            connect(
                &self.state,
                room.workspace,
                room.member,
                surface,
                &format!("{surface}-token-{secret}"),
                Some(secret),
            )
            .await;
        }
    }

    async fn link_slack(&self, room: &Room, channel: &str) {
        self.store
            .link_slack_channel(NewSlackChannelLink {
                slack_channel_id: channel.into(),
                workspace_id: room.workspace,
                channel_id: room.channel,
                thread_id: room.thread,
                member_id: room.member,
            })
            .await
            .unwrap();
    }

    async fn link_github(&self, room: &Room, repo: &str, issue: i64) {
        self.store
            .link_github_issue(NewGithubIssueLink {
                repo: repo.into(),
                issue_number: issue,
                workspace_id: room.workspace,
                channel_id: room.channel,
                thread_id: room.thread,
                member_id: room.member,
            })
            .await
            .unwrap();
    }

    async fn bodies(&self, room: &Room) -> Vec<String> {
        self.store
            .list_messages(room.thread, 50)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect()
    }

    async fn post_slack(&self, path: &str, secret: &str, body: &Value) -> reqwest::Response {
        let body = body.to_string();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        self.http
            .post(format!("{}{path}", self.base))
            .header("x-slack-request-timestamp", &timestamp)
            .header(
                "x-slack-signature",
                slack_signature(secret, &timestamp, &body),
            )
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap()
    }

    async fn post_github(
        &self,
        path: &str,
        secret: &str,
        event: &str,
        body: &Value,
    ) -> reqwest::Response {
        let body = body.to_string();
        self.http
            .post(format!("{}{path}", self.base))
            .header("x-github-event", event)
            .header("x-hub-signature-256", sign_payload(secret, &body))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap()
    }
}

fn slack_url(room: &Room) -> String {
    format!("/workspaces/{}/connections/slack/events", room.workspace.0)
}

fn github_url(room: &Room) -> String {
    format!(
        "/workspaces/{}/connections/github/webhook",
        room.workspace.0
    )
}

fn slack_message(channel: &str, event_id: &str, text: &str) -> Value {
    json!({
        "type": "event_callback",
        "team_id": "T1",
        "event_id": event_id,
        "event": { "type": "message", "channel": channel, "user": "U1", "text": text }
    })
}

fn issue_comment(repo: &str, issue: i64, comment_id: i64, text: &str) -> Value {
    json!({
        "action": "created",
        "repository": { "full_name": repo },
        "issue": { "number": issue },
        "comment": {
            "id": comment_id,
            "body": text,
            "author_association": "OWNER",
            "user": { "login": "octocat" }
        }
    })
}

fn closed_unmerged(repo: &str, number: i64) -> Value {
    json!({
        "action": "closed",
        "repository": { "full_name": repo },
        "pull_request": {
            "number": number,
            "merged": false,
            "title": "Rename",
            "closed_at": "2026-11-23T17:00:00Z"
        }
    })
}

fn has(bodies: &[String], needle: &str) -> bool {
    bodies.iter().any(|b| b.contains(needle))
}

#[tokio::test]
async fn a_delivery_to_a_workspaces_own_url_reaches_its_own_links_and_not_anothers_to_the_same_channel_and_issue(
) {
    let h = harness(false).await;
    let ada = h.signed_up("ada").await;
    let bo = h.signed_up("bo").await;
    h.connect_both(&ada, ADA_SECRET).await;
    h.connect_both(&bo, BO_SECRET).await;
    // Both link the same Slack channel and the same GitHub issue.
    for room in [&ada, &bo] {
        h.link_slack(room, "C1").await;
        h.link_github(room, "o/r", 7).await;
    }

    let res = h
        .post_slack(
            &slack_url(&ada),
            ADA_SECRET,
            &slack_message("C1", "Ev1", "hello ada"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = h
        .post_github(
            &github_url(&ada),
            ADA_SECRET,
            "issue_comment",
            &issue_comment("o/r", 7, 501, "a comment for ada"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = h
        .post_github(
            &github_url(&ada),
            ADA_SECRET,
            "pull_request",
            &closed_unmerged("o/r", 7),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let ada_bodies = h.bodies(&ada).await;
    assert!(has(&ada_bodies, "hello ada"), "{ada_bodies:?}");
    assert!(has(&ada_bodies, "a comment for ada"), "{ada_bodies:?}");
    assert!(has(&ada_bodies, "without merging"), "{ada_bodies:?}");
    assert!(
        h.bodies(&bo).await.is_empty(),
        "another workspace's link to the same channel and issue heard of it"
    );

    // And the other way round, with its own secret.
    let res = h
        .post_slack(
            &slack_url(&bo),
            BO_SECRET,
            &slack_message("C1", "Ev2", "hello bo"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(has(&h.bodies(&bo).await, "hello bo"));
    assert!(!has(&h.bodies(&ada).await, "hello bo"));
}

#[tokio::test]
async fn a_delivery_signed_with_another_workspaces_secret_is_refused() {
    let h = harness(false).await;
    let ada = h.signed_up("ada").await;
    let bo = h.signed_up("bo").await;
    h.connect_both(&ada, ADA_SECRET).await;
    h.connect_both(&bo, BO_SECRET).await;
    h.link_slack(&ada, "C1").await;
    h.link_github(&ada, "o/r", 7).await;

    let res = h
        .post_slack(
            &slack_url(&ada),
            BO_SECRET,
            &slack_message("C1", "Ev1", "forged"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = h
        .post_github(
            &github_url(&ada),
            BO_SECRET,
            "issue_comment",
            &issue_comment("o/r", 7, 502, "forged"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    // A missing signature is the same refusal.
    let res = h
        .http
        .post(format!("{}{}", h.base, github_url(&ada)))
        .header("x-github-event", "ping")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(h.bodies(&ada).await.is_empty());
}

#[tokio::test]
async fn no_secret_no_connection_and_an_unknown_workspace_answer_the_same_404() {
    let h = harness(false).await;
    let with_secret = h.signed_up("ada").await;
    h.connect_both(&with_secret, ADA_SECRET).await;
    // Connected, but stored no inbound secret.
    let no_secret = h.signed_up("bo").await;
    for surface in [ConnectionSurface::Github, ConnectionSurface::Slack] {
        connect(
            &h.state,
            no_secret.workspace,
            no_secret.member,
            surface,
            "a-token-without-an-inbound-secret",
            None,
        )
        .await;
    }
    // Nothing connected at all.
    let no_connection = h.signed_up("cy").await;
    let unknown = format!("/workspaces/{}", uuid::Uuid::now_v7());

    let github_paths = [
        github_url(&no_secret),
        github_url(&no_connection),
        format!("{unknown}/connections/github/webhook"),
    ];
    let slack_paths = [
        slack_url(&no_secret),
        slack_url(&no_connection),
        format!("{unknown}/connections/slack/events"),
    ];
    let mut answers = Vec::new();
    for path in &github_paths {
        let res = h
            .post_github(path, ADA_SECRET, "ping", &json!({ "zen": "x" }))
            .await;
        answers.push((res.status(), res.text().await.unwrap()));
    }
    for path in &slack_paths {
        let res = h
            .post_slack(
                path,
                ADA_SECRET,
                &json!({ "type": "url_verification", "challenge": "c" }),
            )
            .await;
        answers.push((res.status(), res.text().await.unwrap()));
    }
    for (status, _) in &answers {
        assert_eq!(*status, StatusCode::NOT_FOUND);
    }
    assert!(
        answers.windows(2).all(|pair| pair[0].1 == pair[1].1),
        "the three cases tell themselves apart: {answers:?}"
    );
}

#[tokio::test]
async fn slack_url_verification_answers_the_challenge_only_with_a_valid_signature() {
    let h = harness(false).await;
    let ada = h.signed_up("ada").await;
    h.connect_both(&ada, ADA_SECRET).await;
    let challenge = json!({ "type": "url_verification", "challenge": "3eZbrw1aBm2rZgRNFdxV2595E9CY3gmdALWMmHkvFXO7tYXAYM8P" });

    let res = h.post_slack(&slack_url(&ada), ADA_SECRET, &challenge).await;
    assert_eq!(res.status(), StatusCode::OK);
    let answer: Value = res.json().await.unwrap();
    assert_eq!(
        answer["challenge"],
        "3eZbrw1aBm2rZgRNFdxV2595E9CY3gmdALWMmHkvFXO7tYXAYM8P"
    );

    let res = h.post_slack(&slack_url(&ada), BO_SECRET, &challenge).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(!res.text().await.unwrap().contains("3eZbrw1a"));
    // A stale timestamp is refused, signed or not.
    let body = challenge.to_string();
    let stale = (chrono::Utc::now().timestamp() - 3600).to_string();
    let res = h
        .http
        .post(format!("{}{}", h.base, slack_url(&ada)))
        .header("x-slack-request-timestamp", &stale)
        .header(
            "x-slack-signature",
            slack_signature(ADA_SECRET, &stale, &body),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_servers_route_reaches_an_operator_workspace_without_its_own_connection_and_no_other() {
    let h = harness(true).await;
    let operator = h.operator_workspace("ops").await;
    let connected_operator = h.operator_workspace("ops-two").await;
    let signup = h.signed_up("ada").await;
    // The sign-up workspace linked while it had a connection, then removed it,
    // so its link outlives anything that serves it.
    connect(
        &h.state,
        signup.workspace,
        signup.member,
        ConnectionSurface::Slack,
        "ada-slack-token",
        None,
    )
    .await;
    connect(
        &h.state,
        signup.workspace,
        signup.member,
        ConnectionSurface::Github,
        "ada-github-token",
        None,
    )
    .await;
    connect(
        &h.state,
        connected_operator.workspace,
        connected_operator.member,
        ConnectionSurface::Slack,
        "ops-two-slack-token",
        Some(BO_SECRET),
    )
    .await;
    connect(
        &h.state,
        connected_operator.workspace,
        connected_operator.member,
        ConnectionSurface::Github,
        "ops-two-github-token",
        Some(BO_SECRET),
    )
    .await;
    for room in [&operator, &connected_operator, &signup] {
        h.link_slack(room, "C1").await;
        h.link_github(room, "o/r", 7).await;
    }
    for surface in [ConnectionSurface::Slack, ConnectionSurface::Github] {
        h.store
            .delete_secret(signup.workspace, surface.secret_name())
            .await
            .unwrap();
    }

    let res = h
        .post_slack(
            "/integrations/slack/events",
            INSTANCE_SLACK_SECRET,
            &slack_message("C1", "Ev1", "for the operator"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = h
        .post_github(
            "/integrations/github/events",
            INSTANCE_GITHUB_SECRET,
            "issue_comment",
            &issue_comment("o/r", 7, 601, "from github"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let operator_bodies = h.bodies(&operator).await;
    assert!(
        has(&operator_bodies, "for the operator"),
        "{operator_bodies:?}"
    );
    assert!(has(&operator_bodies, "from github"), "{operator_bodies:?}");
    assert!(
        h.bodies(&connected_operator).await.is_empty(),
        "the server's route reached a workspace that connected its own account"
    );
    assert!(
        h.bodies(&signup).await.is_empty(),
        "the server's route reached a workspace the operator did not open"
    );
}

#[tokio::test]
async fn an_event_made_by_the_workspaces_own_account_is_not_echoed_back() {
    let h = harness(false).await;
    let ada = h.signed_up("ada").await;
    let own = Arc::new(MockGithub::default());
    let factory = Arc::new(maidan_server::test_support::MockConnections::default());
    factory.github_for("ada-github-token", own.clone());
    let mut state = h.state.clone();
    state.attach_connection_factory(factory);
    connect(
        &state,
        ada.workspace,
        ada.member,
        ConnectionSurface::Github,
        "ada-github-token",
        Some(ADA_SECRET),
    )
    .await;
    h.link_github(&ada, "o/r", 7).await;
    h.store
        .allow_egress_target(NewEgressTarget {
            workspace_id: ada.workspace,
            surface: EgressSurface::Github,
            selector: "o/r".into(),
            created_by: None,
        })
        .await
        .unwrap();

    // A message in the thread goes out as a comment through the workspace's
    // own client, which takes a receipt for the comment it made.
    let (message, _) = h
        .store
        .post_message_with_event(
            NewMessage {
                thread_id: ada.thread,
                author_id: ada.member,
                body: "shipping it".into(),
                metadata: json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    route_message_to_github(&state, 1, ada.thread, &message).await;
    assert_eq!(egress_worker::sweep_once(&state).await.sent, 1);
    assert_eq!(*own.sent.lock().unwrap(), vec![7]);

    // GitHub sends that comment back. It is not posted into the thread again.
    let before = h.bodies(&ada).await.len();
    let res = h
        .post_github(
            &github_url(&ada),
            ADA_SECRET,
            "issue_comment",
            &issue_comment("o/r", 7, 9007, "shipping it"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        h.bodies(&ada).await.len(),
        before,
        "the comment came back in"
    );
    // Someone else's comment on the same issue still is.
    let res = h
        .post_github(
            &github_url(&ada),
            ADA_SECRET,
            "issue_comment",
            &issue_comment("o/r", 7, 9100, "looks good"),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(has(&h.bodies(&ada).await, "looks good"));

    // Slack: a message a bot made is skipped, as on the server's route.
    connect(
        &state,
        ada.workspace,
        ada.member,
        ConnectionSurface::Slack,
        "ada-slack-token",
        Some(ADA_SECRET),
    )
    .await;
    h.link_slack(&ada, "C1").await;
    let mut bot = slack_message("C1", "Ev9", "from our own bot");
    bot["event"]["bot_id"] = json!("B1");
    let res = h.post_slack(&slack_url(&ada), ADA_SECRET, &bot).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(!has(&h.bodies(&ada).await, "from our own bot"));
}
