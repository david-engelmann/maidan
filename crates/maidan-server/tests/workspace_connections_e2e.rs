//! A workspace connects its own GitHub, Slack and Linear, and a send for a
//! workspace goes through the credential that serves it: its own when it
//! stored one, else the server's for a workspace its operator opened, else
//! nothing. The server's credential is never used for a workspace it does not
//! serve, and no workspace's client acts for another.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use maidan_artifacts::LocalFsStore;
use maidan_auth::capability;
use maidan_bus::InMemoryBus;
use maidan_server::github::{route_message_to_github, GithubError, GithubSender};
use maidan_server::test_support::{tenant, token, MockConnections};
use maidan_server::{egress_worker, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use reqwest::StatusCode;
use serde_json::{json, Value};

/// Records the issues it posted to, and the repository of each.
#[derive(Default)]
struct MockGithub {
    sent: Mutex<Vec<(String, i64)>>,
}

impl MockGithub {
    fn sent(&self) -> Vec<(String, i64)> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl GithubSender for MockGithub {
    async fn post_comment(
        &self,
        repo: &str,
        issue_number: i64,
        _text: &str,
    ) -> Result<Option<ExternalRef>, GithubError> {
        self.sent.lock().unwrap().push((repo.into(), issue_number));
        Ok(Some(ExternalRef::Github {
            repo: repo.into(),
            comment_id: issue_number,
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
    /// The server's own GitHub credential.
    instance: Arc<MockGithub>,
    connections: Arc<MockConnections>,
}

async fn harness() -> Harness {
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
    let instance = Arc::new(MockGithub::default());
    state.attach_github_sender(instance.clone());
    let connections = Arc::new(MockConnections::default());
    state.attach_connection_factory(connections.clone());
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
        instance,
        connections,
    }
}

/// A workspace opened by sign-up, its admin, and the admin's bearer.
struct Tenant {
    workspace: WorkspaceId,
    token: String,
    member: MemberId,
}

impl Harness {
    async fn signed_up(&self, who: &str) -> Tenant {
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
        let workspace = outcome.workspace.id;
        let member = outcome.member.id;
        Tenant {
            workspace,
            member,
            token: token(self.store.as_ref(), workspace, member, capability::all()).await,
        }
    }

    async fn send(
        &self,
        method: reqwest::Method,
        who: &Tenant,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, String) {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&who.token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.unwrap();
        let status = res.status();
        (status, res.text().await.unwrap_or_default())
    }

    /// Connect GitHub with `token`, which the provider knows as `account` and
    /// whose client is `sender`.
    async fn connect_github(&self, who: &Tenant, token: &str, sender: Arc<MockGithub>) {
        self.connections
            .account(token, &token.replace("token", "login"));
        self.connections.github_for(token, sender);
        let (status, text) = self
            .send(
                reqwest::Method::PUT,
                who,
                &format!("/workspaces/{}/connections/github", who.workspace.0),
                Some(json!({ "token": token })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{text}");
    }

    /// A thread in a channel of the workspace, and its id.
    async fn thread(&self, workspace: WorkspaceId, member: MemberId) -> (ChannelId, ThreadId) {
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
        (channel.id, thread.id)
    }

    /// Say something in a thread linked to GitHub and queue it for delivery.
    async fn say(&self, member: MemberId, thread: ThreadId, log_id: i64) {
        let (message, _) = self
            .store
            .post_message_with_event(
                NewMessage {
                    thread_id: thread,
                    author_id: member,
                    body: "shipping it".into(),
                    metadata: json!({}),
                    content: None,
                },
                None,
            )
            .await
            .unwrap();
        route_message_to_github(&self.state, log_id, thread, &message).await;
    }
}

const GITHUB_FIRST: &str = "connect GitHub for this workspace first (Tools, Integrations)";

#[tokio::test]
async fn a_signed_up_workspace_with_its_own_github_blesses_links_watches_and_delivers_through_its_own_client(
) {
    let h = harness().await;
    let ada = h.signed_up("ada").await;
    let own = Arc::new(MockGithub::default());
    h.connect_github(&ada, "ada-token", own.clone()).await;
    let ws = ada.workspace.0;
    let (channel, thread) = h.thread(ada.workspace, ada.member).await;

    let (status, text) = h
        .send(
            reqwest::Method::POST,
            &ada,
            &format!("/workspaces/{ws}/egress-targets"),
            Some(json!({ "surface": "github", "selector": "ada/repo" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    let (status, text) = h
        .send(
            reqwest::Method::POST,
            &ada,
            &format!("/workspaces/{ws}/github-links"),
            Some(json!({ "repo": "ada/repo", "issue_number": 7, "thread_id": thread.0 })),
        )
        .await;
    assert!(status.is_success(), "{status} {text}");
    let (status, text) = h
        .send(
            reqwest::Method::POST,
            &ada,
            &format!("/workspaces/{ws}/pull-watches"),
            Some(json!({ "repo": "ada/repo", "pull_number": 3, "channel_id": channel.0 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");

    h.say(ada.member, thread, 1).await;
    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!(stats.sent, 1, "{stats:?}");
    assert_eq!(own.sent(), vec![("ada/repo".to_string(), 7)]);
    assert!(
        h.instance.sent().is_empty(),
        "the server's own credential acted for a signed-up workspace"
    );
}

#[tokio::test]
async fn a_signed_up_workspace_with_no_connection_is_refused_and_nothing_is_sent() {
    let h = harness().await;
    let ada = h.signed_up("ada").await;
    let ws = ada.workspace.0;
    let (channel, thread) = h.thread(ada.workspace, ada.member).await;
    for (route, body) in [
        (
            "egress-targets",
            json!({ "surface": "github", "selector": "ada/repo" }),
        ),
        (
            "github-links",
            json!({ "repo": "ada/repo", "issue_number": 7, "thread_id": thread.0 }),
        ),
        (
            "pull-watches",
            json!({ "repo": "ada/repo", "pull_number": 3, "channel_id": channel.0 }),
        ),
    ] {
        let (status, text) = h
            .send(
                reqwest::Method::POST,
                &ada,
                &format!("/workspaces/{ws}/{route}"),
                Some(body),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{route}: {text}");
        assert!(text.contains(GITHUB_FIRST), "{route}: {text}");
    }

    // A row queued while something served the workspace is dead-lettered once
    // nothing does, and the server's credential does not pick it up.
    h.store
        .enqueue_egress(NewEgressOutbox {
            workspace_id: ada.workspace,
            thread_id: thread,
            source_log_id: 1,
            target: EgressTarget::Github {
                repo: "ada/repo".into(),
                issue_number: 7,
            },
            body: "left over".into(),
            kind: EgressKind::Projector,
        })
        .await
        .unwrap();
    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!(stats.dead, 1, "{stats:?}");
    assert_eq!(stats.sent, 0, "{stats:?}");
    assert!(h.instance.sent().is_empty());
}

#[tokio::test]
async fn two_workspaces_with_different_tokens_never_use_each_others_client() {
    let h = harness().await;
    let ada = h.signed_up("ada").await;
    let bo = h.signed_up("bo").await;
    let ada_client = Arc::new(MockGithub::default());
    let bo_client = Arc::new(MockGithub::default());
    h.connect_github(&ada, "ada-token", ada_client.clone())
        .await;
    h.connect_github(&bo, "bo-token", bo_client.clone()).await;
    for (who, repo, issue, log_id) in [(&ada, "ada/repo", 11, 1), (&bo, "bo/repo", 22, 2)] {
        let (_, thread) = h.thread(who.workspace, who.member).await;
        let (status, text) = h
            .send(
                reqwest::Method::POST,
                who,
                &format!("/workspaces/{}/egress-targets", who.workspace.0),
                Some(json!({ "surface": "github", "selector": repo })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{text}");
        let (status, text) = h
            .send(
                reqwest::Method::POST,
                who,
                &format!("/workspaces/{}/github-links", who.workspace.0),
                Some(json!({ "repo": repo, "issue_number": issue, "thread_id": thread.0 })),
            )
            .await;
        assert!(status.is_success(), "{status} {text}");
        h.say(who.member, thread, log_id).await;
    }
    let stats = egress_worker::sweep_once(&h.state).await;
    assert_eq!(stats.sent, 2, "{stats:?}");
    assert_eq!(ada_client.sent(), vec![("ada/repo".to_string(), 11)]);
    assert_eq!(bo_client.sent(), vec![("bo/repo".to_string(), 22)]);
    assert!(h.instance.sent().is_empty());
}

#[tokio::test]
async fn an_operator_workspace_uses_the_servers_credential_until_it_connects_its_own() {
    let h = harness().await;
    let ops = tenant(h.store.as_ref(), "ops").await;
    let ops = Tenant {
        workspace: ops.workspace_id,
        token: ops.token,
        member: ops.admin_id,
    };
    let ws = ops.workspace.0;
    let (_, thread) = h.thread(ops.workspace, ops.member).await;
    let (status, text) = h
        .send(
            reqwest::Method::POST,
            &ops,
            &format!("/workspaces/{ws}/egress-targets"),
            Some(json!({ "surface": "github", "selector": "ops/repo" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    let (status, text) = h
        .send(
            reqwest::Method::POST,
            &ops,
            &format!("/workspaces/{ws}/github-links"),
            Some(json!({ "repo": "ops/repo", "issue_number": 5, "thread_id": thread.0 })),
        )
        .await;
    assert!(status.is_success(), "{status} {text}");

    // No connection of its own: the server's credential serves it.
    let (_, listed) = h
        .send(
            reqwest::Method::GET,
            &ops,
            &format!("/workspaces/{ws}/connections"),
            None,
        )
        .await;
    let listed: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed[0]["source"], json!("instance"), "{listed}");
    h.say(ops.member, thread, 1).await;
    assert_eq!(egress_worker::sweep_once(&h.state).await.sent, 1);
    assert_eq!(h.instance.sent(), vec![("ops/repo".to_string(), 5)]);

    // Its own, once stored, is used instead, always.
    let own = Arc::new(MockGithub::default());
    h.connect_github(&ops, "ops-token", own.clone()).await;
    h.say(ops.member, thread, 2).await;
    assert_eq!(egress_worker::sweep_once(&h.state).await.sent, 1);
    assert_eq!(own.sent(), vec![("ops/repo".to_string(), 5)]);
    assert_eq!(
        h.instance.sent().len(),
        1,
        "the server's credential acted after the workspace connected its own"
    );
}

#[tokio::test]
async fn a_token_the_provider_refuses_stores_nothing_and_a_stored_one_is_never_returned() {
    let h = harness().await;
    let ada = h.signed_up("ada").await;
    let ws = ada.workspace.0;
    let path = format!("/workspaces/{ws}/connections");

    let (status, text) = h
        .send(
            reqwest::Method::PUT,
            &ada,
            &format!("{path}/github"),
            Some(json!({ "token": "wrong-token" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(
        text.contains("GitHub refused this token") && text.contains("Bad credentials"),
        "{text}"
    );
    assert!(h
        .store
        .get_secret_ciphertext(ada.workspace, "maidan:github_connection")
        .await
        .unwrap()
        .is_none());
    let (_, listed) = h.send(reqwest::Method::GET, &ada, &path, None).await;
    let listed: Value = serde_json::from_str(&listed).unwrap();
    let rows = listed.as_array().unwrap();
    let surfaces: Vec<_> = rows.iter().map(|r| r["surface"].clone()).collect();
    assert_eq!(surfaces, [json!("github"), json!("slack"), json!("linear")]);
    assert!(rows.iter().all(|r| r["source"].is_null()), "{listed}");
    assert!(rows[2]["inbound"].is_null(), "{listed}");

    // An unknown surface is not a route, and an inbound secret needs a token
    // already stored.
    let (status, _) = h
        .send(
            reqwest::Method::PUT,
            &ada,
            &format!("{path}/discord"),
            Some(json!({ "token": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, text) = h
        .send(
            reqwest::Method::PUT,
            &ada,
            &format!("{path}/github"),
            Some(json!({ "webhook_secret": "0123456789abcdef0123" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");

    let own = Arc::new(MockGithub::default());
    h.connect_github(&ada, "ada-secret-token", own).await;
    let (status, text) = h
        .send(
            reqwest::Method::PUT,
            &ada,
            &format!("{path}/github"),
            Some(json!({ "webhook_secret": "short" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let (status, text) = h
        .send(
            reqwest::Method::PUT,
            &ada,
            &format!("{path}/github"),
            Some(json!({ "webhook_secret": "0123456789abcdef0123" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let (_, listed) = h.send(reqwest::Method::GET, &ada, &path, None).await;
    assert!(!listed.contains("ada-secret-token"), "{listed}");
    assert!(!listed.contains("0123456789abcdef0123"), "{listed}");
    let listed: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed[0]["source"], json!("workspace"));
    assert_eq!(listed[0]["account"], json!("ada-secret-login"));
    assert_eq!(listed[0]["inbound"]["secret_set"], json!(true), "{listed}");
    assert!(listed[0]["inbound"]["url"]
        .as_str()
        .unwrap()
        .ends_with(&format!("/workspaces/{ws}/connections/github/webhook")));
    assert_eq!(listed[1]["inbound"]["secret_set"], json!(false));

    // The token was kept when only the secret changed.
    assert_eq!(
        h.connections.asked.lock().unwrap().len(),
        2,
        "the provider was asked about the two tokens, not about a secret-only update"
    );

    // Removing it ends service, and removing nothing is not an error.
    let (status, _) = h
        .send(
            reqwest::Method::DELETE,
            &ada,
            &format!("{path}/github"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = h
        .send(
            reqwest::Method::DELETE,
            &ada,
            &format!("{path}/github"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, listed) = h.send(reqwest::Method::GET, &ada, &path, None).await;
    let listed: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed[0]["source"], Value::Null, "{listed}");
    let (status, text) = h
        .send(
            reqwest::Method::POST,
            &ada,
            &format!("/workspaces/{ws}/egress-targets"),
            Some(json!({ "surface": "github", "selector": "ada/repo" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(text.contains(GITHUB_FIRST), "{text}");
}

#[tokio::test]
async fn the_generic_secret_routes_cannot_reach_a_connection() {
    let h = harness().await;
    let ada = h.signed_up("ada").await;
    h.connect_github(&ada, "ada-token", Arc::new(MockGithub::default()))
        .await;
    let ws = ada.workspace.0;
    let (status, text) = h
        .send(
            reqwest::Method::GET,
            &ada,
            &format!("/workspaces/{ws}/secrets"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(!text.contains("github_connection"), "{text}");
}

/// Store a connection that cannot be opened (the stored text does not
/// decrypt), the way a rotated key or a damaged row would leave one.
async fn store_unreadable_connection(h: &Harness, who: &Tenant, surface: ConnectionSurface) {
    h.store
        .create_secret(NewSecret {
            workspace_id: who.workspace,
            name: surface.secret_name().into(),
            value_ciphertext: "not a ciphertext this instance can open".into(),
            created_by: who.member,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_github_message_for_a_connection_that_cannot_be_opened_is_queued_not_dropped() {
    let h = harness().await;
    let ada = h.signed_up("ada").await;
    store_unreadable_connection(&h, &ada, ConnectionSurface::Github).await;
    let (_, thread) = h.thread(ada.workspace, ada.member).await;
    h.store
        .link_github_issue(NewGithubIssueLink {
            repo: "ada/repo".into(),
            issue_number: 7,
            workspace_id: ada.workspace,
            channel_id: h.store.get_thread(thread).await.unwrap().channel_id,
            thread_id: thread,
            member_id: ada.member,
        })
        .await
        .unwrap();
    assert!(h.state.github_for(ada.workspace).await.is_err());
    h.say(ada.member, thread, 1).await;
    let queued = h
        .store
        .claim_next_due_egress(chrono::Utc::now(), 60)
        .await
        .unwrap()
        .expect("the message was dropped at routing");
    assert_eq!(queued.workspace_id, ada.workspace);
    assert_eq!(queued.surface, "github");
}

#[tokio::test]
async fn a_slack_message_for_a_connection_that_cannot_be_opened_is_queued_not_dropped() {
    let h = harness().await;
    let ada = h.signed_up("ada").await;
    store_unreadable_connection(&h, &ada, ConnectionSurface::Slack).await;
    let (channel, thread) = h.thread(ada.workspace, ada.member).await;
    h.store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: "C1".into(),
            workspace_id: ada.workspace,
            channel_id: channel,
            thread_id: thread,
            member_id: ada.member,
        })
        .await
        .unwrap();
    assert!(h.state.slack_for(ada.workspace).await.is_err());
    let (message, _) = h
        .store
        .post_message_with_event(
            NewMessage {
                thread_id: thread,
                author_id: ada.member,
                body: "hello slack".into(),
                metadata: json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    maidan_server::slack::route_message_to_slack(&h.state, 1, thread, &message).await;
    let queued = h
        .store
        .claim_next_due_egress(chrono::Utc::now(), 60)
        .await
        .unwrap()
        .expect("the message was dropped at routing");
    assert_eq!(queued.workspace_id, ada.workspace);
    assert_eq!(queued.surface, "slack");
}

#[tokio::test]
async fn dropping_a_state_with_a_pull_refresher_frees_its_store() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::for_tests(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(16)),
        search,
    );
    // Attaching a GitHub sender installs the refresher on the MCP server.
    state.attach_github_sender(Arc::new(MockGithub::default()));
    assert!(Arc::strong_count(&store) > 1);
    drop(state);
    assert_eq!(
        Arc::strong_count(&store),
        1,
        "the state's drop left something holding the store: a loop through the MCP server"
    );
}
