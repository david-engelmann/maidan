//! The console's Deliveries tab against the server: the workspace-wide read of
//! where results went, a signed-in person's replay of one, and the egress
//! allowlist's console routes.
//!
//! The allowlist is the trust boundary for result delivery, so its console
//! routes are tested for who they keep out as much as for who they let in: a
//! person without an admin token, an agent, a session made from a token, a
//! bearer without `token:admin`, and another workspace's admin.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::capability;
use maidan_server::{router, session::SessionSettings, test_support, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    status, ApiTokenId, ChannelMemberRole, EgressSurface, EgressTarget, MemberId, MemberKind,
    NewChannel, NewMaidanSession, NewThread, ResultDelivery, ThreadId, WorkspaceId,
};
use reqwest::{header, Method, StatusCode};
use serde_json::{json, Value};

const SECRET: &[u8] = b"console-deliveries-e2e-session-secret-32b";

struct Env {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Env {
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
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.sessions = Some(SessionSettings {
        secret: Arc::from(SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Env {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        _dir: dir,
    }
}

impl Env {
    /// The cookie of a session `member` signed in to, or one made from
    /// `token` when given.
    async fn signed_in(
        &self,
        workspace: WorkspaceId,
        member: MemberId,
        token: Option<ApiTokenId>,
    ) -> String {
        let session = self
            .store
            .create_session(NewMaidanSession {
                workspace_id: workspace,
                member_id: member,
                api_token_id: token,
                oidc_identity_id: None,
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            })
            .await
            .unwrap();
        let mut headers = axum::http::HeaderMap::new();
        maidan_server::session::set_session_cookie(&mut headers, session.id, 3600, false, SECRET)
            .unwrap();
        headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    /// A console request on a session cookie, from this origin.
    async fn ui(
        &self,
        cookie: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .client
            .request(method, format!("{}/ui/api{path}", self.base))
            .header(header::COOKIE, cookie)
            .header("sec-fetch-site", "same-origin");
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    async fn bearer(
        &self,
        token: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.unwrap();
        let status = res.status();
        (status, res.json().await.unwrap_or(Value::Null))
    }

    /// A task in a new channel of `workspace`.
    async fn task(&self, workspace: WorkspaceId, channel: &str, private: bool) -> ThreadId {
        let channel = self
            .store
            .create_channel(NewChannel {
                workspace_id: workspace,
                name: channel.into(),
                topic: None,
                private,
            })
            .await
            .unwrap();
        self.store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("Ship the widgets".into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap()
            .id
    }

    /// `thread`'s result aimed at `target`, recorded as `outcome` with `why`.
    async fn delivery(
        &self,
        thread: ThreadId,
        surface: EgressSurface,
        selector: &str,
        outcome: &str,
        why: &str,
    ) -> ResultDelivery {
        let target = EgressTarget::parse(surface, selector).expect("a deliverable target");
        let row = self
            .store
            .arm_result_delivery(thread, &target, chrono::Utc::now())
            .await
            .unwrap()
            .expect("armed");
        match outcome {
            "failed" => {
                self.store
                    .mark_result_delivery_failed(row.id, why)
                    .await
                    .unwrap();
            }
            "skipped" => {
                self.store
                    .mark_result_delivery_skipped(row.id, why)
                    .await
                    .unwrap();
            }
            _ => {}
        }
        row
    }
}

fn ids(rows: &Value) -> Vec<String> {
    rows.as_array()
        .expect("a list")
        .iter()
        .map(|r| r["id"].as_str().expect("an id").to_string())
        .collect()
}

#[tokio::test]
async fn the_workspace_list_shows_only_this_workspaces_deliveries_and_filters_by_status() {
    let env = spawn().await;
    let ours = test_support::tenant(env.store.as_ref(), "ours").await;
    let theirs = test_support::tenant(env.store.as_ref(), "theirs").await;
    let task = env.task(ours.workspace_id, "ship", false).await;
    let failed = env
        .delivery(
            task,
            EgressSurface::Github,
            "acme/widgets#7",
            "failed",
            "GitHub answered 502",
        )
        .await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let skipped = env
        .delivery(
            task,
            EgressSurface::Slack,
            "C0123ABCDEF",
            "skipped",
            "target not in the workspace egress allowlist",
        )
        .await;
    let elsewhere = env.task(theirs.workspace_id, "ship", false).await;
    env.delivery(
        elsewhere,
        EgressSurface::Github,
        "acme/widgets#9",
        "failed",
        "GitHub answered 502",
    )
    .await;
    let path = format!("/workspaces/{}/result-deliveries", ours.workspace_id.0);

    let (code, all) = env.bearer(&ours.token, Method::GET, &path, None).await;
    assert_eq!(code, StatusCode::OK, "{all}");
    assert_eq!(
        ids(&all),
        vec![skipped.id.0.to_string(), failed.id.0.to_string()],
        "this workspace's rows only, the newest change first"
    );
    assert_eq!(all[0]["status"], status::SKIPPED);
    assert_eq!(all[1]["thread_id"], task.0.to_string());

    let (code, only) = env
        .bearer(
            &ours.token,
            Method::GET,
            &format!("{path}?status=failed"),
            None,
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{only}");
    assert_eq!(ids(&only), vec![failed.id.0.to_string()]);
    assert_eq!(only[0]["last_error"], "GitHub answered 502");

    let (code, _) = env
        .bearer(
            &ours.token,
            Method::GET,
            &format!("{path}?status=lost"),
            None,
        )
        .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);

    // Another workspace's admin, holding every capability, reads nothing here.
    let (code, body) = env.bearer(&theirs.token, Method::GET, &path, None).await;
    assert!(
        code == StatusCode::FORBIDDEN || code == StatusCode::NOT_FOUND,
        "{code} {body}"
    );

    // A person signed in reads the same list through the console's route.
    let cookie = env.signed_in(ours.workspace_id, ours.admin_id, None).await;
    let (code, seen) = env
        .ui(
            &cookie,
            Method::GET,
            &format!("/workspaces/{}/result-deliveries", ours.workspace_id.0),
            None,
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{seen}");
    assert_eq!(ids(&seen), ids(&all));
}

#[tokio::test]
async fn a_private_channels_deliveries_stay_out_of_reach_of_a_member_outside_it() {
    let env = spawn().await;
    let ours = test_support::tenant(env.store.as_ref(), "ours").await;
    let carol = ours
        .member(env.store.as_ref(), "carol", MemberKind::Human)
        .await;
    let open = env.task(ours.workspace_id, "ship", false).await;
    let closed = env.task(ours.workspace_id, "board-only", true).await;
    let public = env
        .delivery(
            open,
            EgressSurface::Github,
            "acme/widgets#1",
            "failed",
            "502",
        )
        .await;
    let hidden = env
        .delivery(
            closed,
            EgressSurface::Github,
            "acme/secret#2",
            "failed",
            "502",
        )
        .await;
    let path = format!("/workspaces/{}/result-deliveries", ours.workspace_id.0);

    let (code, rows) = env.bearer(&carol.token, Method::GET, &path, None).await;
    assert_eq!(code, StatusCode::OK, "{rows}");
    assert_eq!(ids(&rows), vec![public.id.0.to_string()]);

    let channel = env.store.get_thread(closed).await.unwrap().channel_id;
    env.store
        .add_channel_member(channel, carol.member_id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let (_, rows) = env.bearer(&carol.token, Method::GET, &path, None).await;
    assert!(ids(&rows).contains(&hidden.id.0.to_string()), "{rows}");
}

#[tokio::test]
async fn a_signed_in_admin_allows_revokes_and_replays_from_the_console() {
    let env = spawn().await;
    let ours = test_support::tenant(env.store.as_ref(), "ours").await;
    let cookie = env.signed_in(ours.workspace_id, ours.admin_id, None).await;
    let targets = format!("/workspaces/{}/egress-targets", ours.workspace_id.0);

    let (code, list) = env.ui(&cookie, Method::GET, &targets, None).await;
    assert_eq!(code, StatusCode::OK, "{list}");
    assert_eq!(list, json!([]), "an empty allowlist delivers nowhere");

    let (code, made) = env
        .ui(
            &cookie,
            Method::POST,
            &targets,
            Some(json!({"surface": "slack", "selector": "C0123ABCDEF"})),
        )
        .await;
    assert_eq!(code, StatusCode::CREATED, "{made}");
    assert_eq!(made["created_by"], ours.admin_id.0.to_string());

    // A shape only the server judges comes back with its reason.
    let (code, refused) = env
        .ui(
            &cookie,
            Method::POST,
            &targets,
            Some(json!({"surface": "github_branch", "selector": "acme/widgets@prod"})),
        )
        .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(
        refused
            .to_string()
            .contains("`prod` is never an allowed base"),
        "{refused}"
    );

    let (_, list) = env.ui(&cookie, Method::GET, &targets, None).await;
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    assert_eq!(list[0]["selector"], "C0123ABCDEF");
    assert_eq!(list[0]["created_by"], ours.admin_id.0.to_string());

    // With the channel allowed, a skipped delivery to it goes out on replay.
    let task = env.task(ours.workspace_id, "ship", false).await;
    let skipped = env
        .delivery(
            task,
            EgressSurface::Slack,
            "C0123ABCDEF",
            "skipped",
            "target not in the workspace egress allowlist",
        )
        .await;
    let (code, replayed) = env
        .ui(
            &cookie,
            Method::POST,
            &format!("/threads/{}/deliveries/{}/replay", task.0, skipped.id.0),
            None,
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{replayed}");
    assert_eq!(replayed["status"], status::PENDING);

    let id = made["id"].as_str().unwrap();
    let (code, _) = env
        .ui(&cookie, Method::DELETE, &format!("{targets}/{id}"), None)
        .await;
    assert_eq!(code, StatusCode::NO_CONTENT);
    let (code, _) = env
        .ui(&cookie, Method::DELETE, &format!("{targets}/{id}"), None)
        .await;
    assert_eq!(code, StatusCode::NOT_FOUND, "a second revoke finds nothing");
    let (_, list) = env.ui(&cookie, Method::GET, &targets, None).await;
    assert_eq!(list, json!([]));

    let audit = env
        .store
        .list_audit_for_workspace(ours.workspace_id, 50)
        .await
        .unwrap();
    for action in ["egress_target.allow", "egress_target.revoke"] {
        let row = audit
            .iter()
            .find(|a| a.action == action)
            .unwrap_or_else(|| panic!("no {action} audit row"));
        assert_eq!(
            row.actor_id,
            Some(ours.admin_id),
            "{action} names the person"
        );
    }
}

#[tokio::test]
async fn only_a_human_admin_signed_in_reaches_the_console_allowlist() {
    let env = spawn().await;
    let ours = test_support::tenant(env.store.as_ref(), "ours").await;
    let theirs = test_support::tenant(env.store.as_ref(), "theirs").await;
    let targets = format!("/workspaces/{}/egress-targets", ours.workspace_id.0);
    let body = json!({"surface": "github", "selector": "acme/widgets"});
    let entry = env
        .bearer(&ours.token, Method::POST, &targets, Some(body.clone()))
        .await
        .1["id"]
        .as_str()
        .expect("the admin's bearer allows")
        .to_string();

    // A person whose own tokens hold no token:admin.
    let bob = env
        .store
        .create_member(maidan_types::NewMember {
            workspace_id: ours.workspace_id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let bob_token = test_support::token(
        env.store.as_ref(),
        ours.workspace_id,
        bob.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
    )
    .await;
    let bob_cookie = env.signed_in(ours.workspace_id, bob.id, None).await;
    // An agent holding token:admin: its own sign-in still never qualifies.
    let agent = ours
        .member(env.store.as_ref(), "worker", MemberKind::Agent)
        .await;
    let agent_cookie = env
        .signed_in(ours.workspace_id, agent.member_id, None)
        .await;
    // A session made from an admin token: that token's holder uses the
    // bearer routes, where its own grant decides.
    let registrar = test_support::registrar(env.store.as_ref(), ours.workspace_id).await;
    let registrar_member = env
        .store
        .list_members(ours.workspace_id)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.handle == "registrar")
        .unwrap()
        .id;
    let token_cookie = env
        .signed_in(ours.workspace_id, registrar_member, Some(registrar))
        .await;
    // Another workspace's admin, signed in to their own.
    let stranger_cookie = env
        .signed_in(theirs.workspace_id, theirs.admin_id, None)
        .await;

    for (who, cookie) in [
        ("a person without an admin token", &bob_cookie),
        ("an agent", &agent_cookie),
        ("a session made from a token", &token_cookie),
        ("another workspace's admin", &stranger_cookie),
    ] {
        let (code, read) = env.ui(cookie, Method::GET, &targets, None).await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{who} listed: {read}");
        let (code, _) = env
            .ui(
                cookie,
                Method::POST,
                &targets,
                Some(json!({"surface": "github", "selector": "evil/repo"})),
            )
            .await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{who} allowed a destination");
        let (code, _) = env
            .ui(cookie, Method::DELETE, &format!("{targets}/{entry}"), None)
            .await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{who} revoked a destination");
    }

    // A bearer on the console route is held to token:admin, as on the bearer route.
    let (code, refused) = env
        .bearer(&bob_token, Method::GET, &format!("/ui/api{targets}"), None)
        .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert!(refused.to_string().contains("token:admin"), "{refused}");
    let (code, _) = env
        .bearer(&ours.token, Method::GET, &format!("/ui/api{targets}"), None)
        .await;
    assert_eq!(code, StatusCode::OK);

    let left = env
        .store
        .list_egress_targets(ours.workspace_id)
        .await
        .unwrap();
    assert_eq!(left.len(), 1, "nothing was added or revoked: {left:?}");
    assert_eq!(left[0].selector, "acme/widgets");
}

#[tokio::test]
async fn a_blessing_keeps_its_first_blesser() {
    let env = spawn().await;
    let ours = test_support::tenant(env.store.as_ref(), "ours").await;
    let second = ours
        .member(env.store.as_ref(), "second", MemberKind::Human)
        .await;
    let targets = format!("/workspaces/{}/egress-targets", ours.workspace_id.0);
    let body = json!({"surface": "github", "selector": "acme/widgets"});
    let (code, first) = env
        .bearer(&ours.token, Method::POST, &targets, Some(body.clone()))
        .await;
    assert_eq!(code, StatusCode::CREATED, "{first}");
    assert_eq!(first["created_by"], ours.admin_id.0.to_string());
    let (_, again) = env
        .bearer(&second.token, Method::POST, &targets, Some(body))
        .await;
    assert_eq!(again["id"], first["id"], "a re-bless is the same entry");
    assert_eq!(
        again["created_by"],
        ours.admin_id.0.to_string(),
        "and keeps who made it"
    );
}
