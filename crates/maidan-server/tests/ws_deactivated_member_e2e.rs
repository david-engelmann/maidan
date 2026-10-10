//! A member deactivated through SCIM loses their open WebSocket streams at the
//! stream's next frame, not only at their next HTTP request (follow-up to
//! #1346). Deactivation revokes tokens and ends sessions on the next request,
//! but a stream makes no further requests, so it outlived both.
//!
//! Two tenants: deactivating Alice in workspace A ends Alice's streams (her
//! signed-in session's and her token's) and nobody else's, in A or in B.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router, subscribe_resume, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{AuditScope, MemberId, NewApiToken, NewAuditEvent, NewWorkspace, WorkspaceId};
use reqwest::redirect::Policy;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::frame::coding::CloseCode, Message},
    MaybeTlsStream, WebSocketStream,
};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const SECRET: &[u8] = b"ws-deactivated-member-e2e-secret-32b!";
/// Longer than the server's gap between two checks of a stream's credential.
const PAST_RECHECK_GAP: Duration = Duration::from_millis(1200);

struct Env {
    addr: SocketAddr,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Env {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::with_capacity(64));
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    state.oidc = Some(Arc::new(OidcRuntime {
        settings: OidcSettings {
            enabled: true,
            mock: true,
            issuer: "https://mock.idp.local".into(),
            redirect_uri: "http://127.0.0.1/auth/oidc/callback".into(),
            auto_provision: true,
            link_email: false,
            session_ttl_secs: 3600,
            pending_ttl_secs: 600,
            cookie_secure: false,
            post_logout_redirect_uri: None,
            first_admin_mint: true,
            auto_mint: false,
        },
        session_secret: Arc::from(SECRET),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Env {
        addr,
        client: reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        _dir: dir,
    }
}

impl Env {
    async fn workspace(&self, name: &str) -> WorkspaceId {
        self.store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap()
            .id
    }

    /// Sign `subject` in to `ws` through the mock provider; the member is
    /// provisioned and given a SCIM link. Returns (cookie, member).
    async fn sign_in(&self, ws: WorkspaceId, subject: &str) -> (String, MemberId) {
        let base = format!("http://{}", self.addr);
        let login = self
            .client
            .get(format!("{base}/auth/oidc/login?workspace_id={}", ws.0))
            .send()
            .await
            .unwrap();
        let location = login.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap()
            .replace("mock_sub=mock-user", &format!("mock_sub={subject}"))
            .replace(
                "mock_email=human@example.com",
                &format!("mock_email={subject}@example.com"),
            );
        let callback = self
            .client
            .get(format!("{base}{location}"))
            .send()
            .await
            .unwrap();
        let cookie = callback
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|s| s.split(';').next())
            .find(|p| p.starts_with("maidan_session="))
            .expect("session cookie")
            .to_string();
        let session: Value = self
            .client
            .get(format!("{base}/auth/session"))
            .header(reqwest::header::COOKIE, &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let member = MemberId(session["member_id"].as_str().unwrap().parse().unwrap());
        self.store
            .create_scim_user(member, ws, None, true)
            .await
            .unwrap();
        (cookie, member)
    }

    async fn token(&self, ws: WorkspaceId, member: MemberId) -> String {
        let secret = TokenSecret::generate();
        self.store
            .create_api_token(NewApiToken {
                workspace_id: ws,
                member_id: member,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: vec![
                    capability::WORKSPACE_READ.into(),
                    capability::EVENT_SUBSCRIBE.into(),
                ],
                expires_at: None,
            })
            .await
            .unwrap();
        secret.as_str().to_string()
    }

    /// Open a presence stream for `member` in `ws`, by cookie or by token, and
    /// read past the subscribe ack.
    async fn stream(
        &self,
        ws: WorkspaceId,
        member: MemberId,
        cookie: Option<&str>,
        token: Option<&str>,
    ) -> Ws {
        let mut req = format!("ws://{}/ws/subscribe", self.addr)
            .into_client_request()
            .unwrap();
        if let Some(cookie) = cookie {
            req.headers_mut()
                .insert("Cookie", cookie.parse().expect("cookie header"));
        }
        let (mut socket, _) = connect_async(req).await.expect("ws connect");
        let mut frame = json!({
            "filter": { "workspace_id": ws.0 },
            "member_id": member.0,
        });
        if let Some(token) = token {
            frame["token"] = json!(token);
        }
        socket.send(Message::Text(frame.to_string())).await.unwrap();
        loop {
            match next(&mut socket).await {
                Frame::Json(v) if v["type"] == "subscribe_ack" => break,
                Frame::Json(_) => {}
                other => panic!("stream refused: {other:?}"),
            }
        }
        socket
    }

    async fn deactivate(&self, ws: WorkspaceId, member: MemberId) {
        self.store
            .scim_update_user_audited(
                ws,
                member,
                None,
                None,
                false,
                NewAuditEvent {
                    scope: AuditScope::Workspace(ws),
                    actor_id: None,
                    action: "scim.user.update".into(),
                    target_kind: Some("member".into()),
                    target_id: Some(member.0),
                    metadata: json!({ "active": false }),
                },
            )
            .await
            .unwrap()
            .expect("SCIM user");
    }
}

#[derive(Debug)]
enum Frame {
    Json(Value),
    Closed(Option<(CloseCode, String)>),
    Silent,
}

async fn next(socket: &mut Ws) -> Frame {
    loop {
        match tokio::time::timeout(Duration::from_secs(3), socket.next()).await {
            Err(_) => return Frame::Silent,
            Ok(None) | Ok(Some(Err(_))) => return Frame::Closed(None),
            Ok(Some(Ok(Message::Close(frame)))) => {
                return Frame::Closed(frame.map(|f| (f.code, f.reason.to_string())))
            }
            Ok(Some(Ok(Message::Text(text)))) => {
                return Frame::Json(serde_json::from_str(&text).unwrap())
            }
            Ok(Some(Ok(_))) => {}
        }
    }
}

/// Read until `pred` matches a frame, the stream closes, or it goes quiet.
async fn until(socket: &mut Ws, pred: impl Fn(&Value) -> bool) -> Frame {
    loop {
        match next(socket).await {
            Frame::Json(v) if pred(&v) => return Frame::Json(v),
            Frame::Json(_) => {}
            other => return other,
        }
    }
}

fn presence_of(member: MemberId, status: &'static str) -> impl Fn(&Value) -> bool {
    move |v: &Value| {
        v["type"] == "presence" && v["member_id"] == member.0.to_string() && v["status"] == status
    }
}

async fn say(socket: &mut Ws, status: &str) {
    socket
        .send(Message::Text(
            json!({ "type": "presence", "status": status }).to_string(),
        ))
        .await
        .unwrap();
}

fn assert_ended(frame: Frame, reason: &str) {
    match frame {
        Frame::Closed(Some((code, why))) => {
            assert_eq!(code, CloseCode::Policy, "{why}");
            assert_eq!(why, reason);
        }
        other => panic!("the stream should have closed with 1008 {reason:?}, got {other:?}"),
    }
}

#[tokio::test]
async fn a_deactivated_members_session_stream_ends_at_its_next_frame() {
    let env = spawn().await;
    let a = env.workspace("Alpha").await;
    let b = env.workspace("Bravo").await;
    let (alice_cookie, alice) = env.sign_in(a, "alice").await;
    let (carol_cookie, carol) = env.sign_in(a, "carol").await;
    let (bob_cookie, bob) = env.sign_in(b, "alice-in-b").await;

    let mut alice_ws = env.stream(a, alice, Some(&alice_cookie), None).await;
    let mut carol_ws = env.stream(a, carol, Some(&carol_cookie), None).await;
    let mut bob_ws = env.stream(b, bob, Some(&bob_cookie), None).await;

    env.deactivate(a, alice).await;
    tokio::time::sleep(PAST_RECHECK_GAP).await;

    // Carol's presence is the next frame bound for Alice's stream: it closes
    // instead of carrying it.
    say(&mut carol_ws, "away").await;
    assert_ended(
        until(&mut alice_ws, presence_of(carol, "away")).await,
        "member deactivated",
    );

    // Nobody else's stream ends: Carol in the same workspace, Bob in another.
    say(&mut carol_ws, "online").await;
    assert!(matches!(
        until(&mut carol_ws, presence_of(carol, "online")).await,
        Frame::Json(_)
    ));
    say(&mut bob_ws, "away").await;
    assert!(matches!(
        until(&mut bob_ws, presence_of(bob, "away")).await,
        Frame::Json(_)
    ));
}

#[tokio::test]
async fn a_deactivated_member_cannot_speak_through_an_open_stream() {
    let env = spawn().await;
    let a = env.workspace("Alpha").await;
    let (alice_cookie, alice) = env.sign_in(a, "alice").await;
    let (carol_cookie, carol) = env.sign_in(a, "carol").await;
    let mut alice_ws = env.stream(a, alice, Some(&alice_cookie), None).await;
    let mut carol_ws = env.stream(a, carol, Some(&carol_cookie), None).await;

    env.deactivate(a, alice).await;
    tokio::time::sleep(PAST_RECHECK_GAP).await;

    say(&mut alice_ws, "away").await;
    assert_ended(until(&mut alice_ws, |_| false).await, "member deactivated");
    // Carol never hears the deactivated member go "away".
    match until(&mut carol_ws, presence_of(alice, "away")).await {
        Frame::Json(v) => panic!("a deactivated member spoke through the stream: {v}"),
        Frame::Silent => {}
        Frame::Closed(c) => panic!("Carol's stream closed: {c:?}"),
    }
}

#[tokio::test]
async fn a_deactivated_members_token_stream_ends_too() {
    let env = spawn().await;
    let a = env.workspace("Alpha").await;
    let b = env.workspace("Bravo").await;
    let (_, alice) = env.sign_in(a, "alice").await;
    let (_, bob) = env.sign_in(b, "bob").await;
    let alice_token = env.token(a, alice).await;
    let bob_token = env.token(b, bob).await;
    let mut alice_ws = env.stream(a, alice, None, Some(&alice_token)).await;
    let mut bob_ws = env.stream(b, bob, None, Some(&bob_token)).await;

    env.deactivate(a, alice).await;
    tokio::time::sleep(PAST_RECHECK_GAP).await;

    say(&mut alice_ws, "away").await;
    // Deactivation also revoked the token, but the re-check looks at the
    // member first, so the stream says why it really ended.
    assert_ended(until(&mut alice_ws, |_| false).await, "member deactivated");
    say(&mut bob_ws, "away").await;
    assert!(matches!(
        until(&mut bob_ws, presence_of(bob, "away")).await,
        Frame::Json(_)
    ));
}
