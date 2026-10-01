//! `POST /auth/session/from-token`: a pasted bearer becomes an `HttpOnly`
//! browser session that holds that token's authority and nothing else.
//!
//! Each request on the session resolves the token again, so the session ends
//! when the token is revoked, rotated or its grant is withdrawn; it carries the
//! token's capabilities (not the fixed set an OIDC session has) and its
//! workspace; and an unsafe request or socket on it from another origin is
//! refused.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use chrono::{Duration as ChronoDuration, Utc};
use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router,
    session::SessionSettings,
    subscribe_resume, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApiTokenId, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace,
    ThreadId, WorkspaceId,
};
use reqwest::{header, redirect::Policy, StatusCode};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

const SESSION_SECRET: &[u8] = b"test-session-secret-32-bytes-min!";

struct Tenant {
    ws: WorkspaceId,
    member: MemberId,
    thread: ThreadId,
    /// `token:admin` plus the everyday capabilities.
    admin: String,
    admin_id: ApiTokenId,
}

struct World {
    addr: SocketAddr,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    a: Tenant,
    b: Tenant,
    _dir: tempfile::TempDir,
}

impl World {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn mint(&self, t: &Tenant, member: MemberId, caps: &[&str]) -> (String, ApiTokenId) {
        mint(self.store.as_ref(), t.ws, member, caps).await
    }

    /// Exchange `bearer` and return the `Cookie` header for its session.
    async fn exchange(&self, bearer: &str) -> String {
        let resp = self
            .client
            .post(self.url("/auth/session/from-token"))
            .bearer_auth(bearer)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "exchange");
        let set = resp
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("a session cookie")
            .to_string();
        assert!(set.contains("HttpOnly"), "{set}");
        assert!(set.contains("SameSite=Lax"), "{set}");
        set.split(';').next().unwrap().to_string()
    }

    async fn get(&self, cookie: &str, path: &str) -> StatusCode {
        self.client
            .get(self.url(path))
            .header(header::COOKIE, cookie)
            .send()
            .await
            .unwrap()
            .status()
    }

    /// `POST /ui/api/workspaces/{ws}/channels` on the session, from `origin`.
    async fn create_channel(
        &self,
        cookie: &str,
        ws: WorkspaceId,
        origin: Option<&str>,
    ) -> reqwest::Response {
        let mut req = self
            .client
            .post(self.url(&format!("/ui/api/workspaces/{}/channels", ws.0)))
            .header(header::COOKIE, cookie)
            .json(&json!({ "name": format!("c-{}", uuid::Uuid::new_v4().simple()) }));
        if let Some(origin) = origin {
            req = req.header(header::ORIGIN, origin);
        }
        req.send().await.unwrap()
    }
}

async fn mint(
    store: &dyn Store,
    ws: WorkspaceId,
    member: MemberId,
    caps: &[&str],
) -> (String, ApiTokenId) {
    let secret = TokenSecret::generate();
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    (secret.as_str().to_string(), token.id)
}

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-human"),
            display_name: None,
            kind: MemberKind::Human,
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
            title: Some("t".into()),
        })
        .await
        .unwrap();
    let (admin, admin_id) = mint(
        store,
        ws.id,
        member.id,
        &[
            capability::WORKSPACE_READ,
            capability::WORKSPACE_WRITE,
            capability::MESSAGE_POST,
            capability::EVENT_SUBSCRIBE,
            capability::TOKEN_ADMIN,
        ],
    )
    .await;
    Tenant {
        ws: ws.id,
        member: member.id,
        thread: thread.id,
        admin,
        admin_id,
    }
}

enum Sessions {
    /// `MAIDAN_SESSION_SECRET` without OIDC.
    Standalone,
    /// A mock OIDC runtime, whose settings the sessions then use.
    Oidc,
    None,
}

async fn world(sessions: Sessions) -> World {
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
    let a = tenant(store.as_ref(), "a").await;
    let b = tenant(store.as_ref(), "b").await;

    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    match sessions {
        Sessions::Standalone => {
            state.sessions = Some(SessionSettings {
                secret: Arc::from(SESSION_SECRET),
                ttl_secs: 3600,
                cookie_secure: false,
            });
        }
        Sessions::Oidc => {
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
                session_secret: Arc::from(SESSION_SECRET),
                client: None,
                http_client: None,
                end_session_url: None,
                logout_client_id: None,
            }));
        }
        Sessions::None => {}
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    World {
        addr,
        client: reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        store,
        a,
        b,
        _dir: dir,
    }
}

#[tokio::test]
async fn a_pasted_token_becomes_a_session_with_that_tokens_authority() {
    let w = world(Sessions::Standalone).await;
    let cookie = w.exchange(&w.a.admin).await;

    let session: Value = w
        .client
        .get(w.url("/auth/session"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["member_id"], json!(w.a.member.0));
    assert_eq!(session["token_id"], json!(w.a.admin_id.0));

    // The bearer routes take the session, with no bearer sent.
    assert_eq!(w.get(&cookie, "/me").await, StatusCode::OK);
    // token:admin, which no OIDC session holds.
    assert_eq!(
        w.get(
            &cookie,
            &format!("/workspaces/{}/members/{}/tokens", w.a.ws.0, w.a.member.0)
        )
        .await,
        StatusCode::OK
    );
    let same = w.create_channel(&cookie, w.a.ws, Some(&w.origin())).await;
    assert_eq!(same.status(), StatusCode::CREATED);

    let recorded = w
        .store
        .list_audit_for_workspace(w.a.ws, 50)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.action == "session.from_token")
        .expect("the exchange is audited");
    assert_eq!(recorded.target_id, Some(w.a.admin_id.0));
    assert_eq!(recorded.actor_id, Some(w.a.member));
}

#[tokio::test]
async fn a_revoked_tokens_session_stops_working() {
    let w = world(Sessions::Standalone).await;
    let (secret, id) = w
        .mint(&w.a, w.a.member, &[capability::WORKSPACE_READ])
        .await;
    let cookie = w.exchange(&secret).await;
    assert_eq!(w.get(&cookie, "/me").await, StatusCode::OK);

    let revoked = w
        .client
        .delete(w.url(&format!("/tokens/{}", id.0)))
        .bearer_auth(&w.a.admin)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::OK);

    assert_eq!(w.get(&cookie, "/me").await, StatusCode::UNAUTHORIZED);
    let ui = format!("/ui/api/workspaces/{}/channels", w.a.ws.0);
    assert_eq!(w.get(&cookie, &ui).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        w.get(&cookie, "/auth/session").await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_rotated_tokens_session_stops_working() {
    let w = world(Sessions::Standalone).await;
    let (secret, id) = w
        .mint(&w.a, w.a.member, &[capability::WORKSPACE_READ])
        .await;
    let cookie = w.exchange(&secret).await;

    let rotated = w
        .client
        .post(w.url(&format!("/tokens/{}/rotate", id.0)))
        .bearer_auth(&secret)
        .send()
        .await
        .unwrap();
    assert_eq!(rotated.status(), StatusCode::OK);
    let successor: Value = rotated.json().await.unwrap();

    assert_eq!(w.get(&cookie, "/me").await, StatusCode::UNAUTHORIZED);
    // The successor's secret makes a new session.
    let fresh = w.exchange(successor["secret"].as_str().unwrap()).await;
    assert_eq!(w.get(&fresh, "/me").await, StatusCode::OK);
}

#[tokio::test]
async fn a_cross_origin_unsafe_request_on_a_session_is_refused() {
    let w = world(Sessions::Standalone).await;
    let cookie = w.exchange(&w.a.admin).await;

    for origin in ["http://evil.example", "null", "http://127.0.0.1:1"] {
        let refused = w.create_channel(&cookie, w.a.ws, Some(origin)).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{origin}");
    }
    // The bearer tree too: a message posted from a sibling origin.
    let refused = w
        .client
        .post(w.url(&format!("/threads/{}/messages", w.a.thread.0)))
        .header(header::COOKIE, &cookie)
        .header("sec-fetch-site", "same-site")
        .json(&json!({ "body": "forged" }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    // This origin, and a client that sends no Origin at all, are served.
    let same = w.create_channel(&cookie, w.a.ws, Some(&w.origin())).await;
    assert_eq!(same.status(), StatusCode::CREATED);
    let bare = w.create_channel(&cookie, w.a.ws, None).await;
    assert_eq!(bare.status(), StatusCode::CREATED);
    // A bearer is not ambient, so its origin is not checked.
    let bearer = w
        .client
        .post(w.url(&format!("/ui/api/workspaces/{}/channels", w.a.ws.0)))
        .bearer_auth(&w.a.admin)
        .header(header::ORIGIN, "http://evil.example")
        .json(&json!({ "name": "by-bearer" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bearer.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn a_cross_origin_socket_cannot_use_a_session() {
    let w = world(Sessions::Standalone).await;
    let cookie = w.exchange(&w.a.admin).await;
    let subscribe = |origin: String| {
        let cookie = cookie.clone();
        let url = format!("ws://{}/ws/subscribe", w.addr);
        let ws = w.a.ws;
        async move {
            let mut req = url.into_client_request().unwrap();
            req.headers_mut().insert("Cookie", cookie.parse().unwrap());
            req.headers_mut().insert("Origin", origin.parse().unwrap());
            let (mut sock, _) = connect_async(req).await.unwrap();
            let frame = json!({ "filter": { "workspace_id": ws.0 }, "after_id": 0 });
            sock.send(Message::Text(frame.to_string())).await.unwrap();
            let reply = tokio::time::timeout(Duration::from_secs(5), sock.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            match reply {
                Message::Text(text) => serde_json::from_str::<Value>(&text).unwrap()["type"]
                    .as_str()
                    .map(String::from),
                Message::Close(_) => None,
                other => panic!("unexpected {other:?}"),
            }
        }
    };
    assert_eq!(
        subscribe(w.origin()).await.as_deref(),
        Some("subscribe_ack")
    );
    assert_eq!(subscribe("http://evil.example".into()).await, None);
}

#[tokio::test]
async fn a_session_from_a_workspace_a_token_cannot_touch_workspace_b() {
    let w = world(Sessions::Standalone).await;
    let cookie = w.exchange(&w.a.admin).await;
    let origin = w.origin();

    let write = w.create_channel(&cookie, w.b.ws, Some(&origin)).await;
    assert!(
        matches!(
            write.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ),
        "{}",
        write.status()
    );
    for path in [
        format!("/ui/api/workspaces/{}/channels", w.b.ws.0),
        format!("/workspaces/{}/channels", w.b.ws.0),
        format!("/workspaces/{}/members/{}/tokens", w.b.ws.0, w.b.member.0),
        format!("/threads/{}/messages", w.b.thread.0),
    ] {
        let status = w.get(&cookie, &path).await;
        assert!(
            matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "{path}: {status}"
        );
    }
    assert!(w
        .store
        .list_channels(w.b.ws)
        .await
        .unwrap()
        .iter()
        .all(|c| c.name == "general"));
}

#[tokio::test]
async fn a_narrowed_tokens_session_has_only_the_narrowed_capabilities() {
    let w = world(Sessions::Standalone).await;
    let narrowed: Value = w
        .client
        .post(w.url("/tokens/attenuate"))
        .bearer_auth(&w.a.admin)
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cookie = w.exchange(narrowed["secret"].as_str().unwrap()).await;

    let ui = format!("/ui/api/workspaces/{}/channels", w.a.ws.0);
    assert_eq!(w.get(&cookie, &ui).await, StatusCode::OK);
    // An OIDC session could create a channel here; the narrowed token cannot.
    let write = w.create_channel(&cookie, w.a.ws, Some(&w.origin())).await;
    assert_eq!(write.status(), StatusCode::FORBIDDEN);
    let body: Value = write.json().await.unwrap();
    assert!(
        body["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("workspace:write"),
        "{body}"
    );
    assert_eq!(
        w.get(
            &cookie,
            &format!("/workspaces/{}/members/{}/tokens", w.a.ws.0, w.a.member.0)
        )
        .await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_delegated_tokens_session_stays_bound_by_its_grant() {
    let w = world(Sessions::Standalone).await;
    let delegate = w
        .store
        .create_member(NewMember {
            workspace_id: w.a.ws,
            handle: "orchestrator".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id;
    let (delegate_token, _) = w
        .mint(
            &w.a,
            delegate,
            &[capability::WORKSPACE_READ, capability::MESSAGE_POST],
        )
        .await;
    let grant: Value = w
        .client
        .post(w.url(&format!("/workspaces/{}/delegation-grants", w.a.ws.0)))
        .bearer_auth(&w.a.admin)
        .json(&json!({
            "subject_id": w.a.member.0,
            "delegate_id": delegate.0,
            "capabilities": [capability::WORKSPACE_READ],
            "purpose": "read for the human",
            "expires_at": Utc::now() + ChronoDuration::hours(1),
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let exchanged: Value = w
        .client
        .post(w.url("/tokens/delegate"))
        .bearer_auth(&delegate_token)
        .json(&json!({ "grant_id": grant["id"] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cookie = w
        .exchange(exchanged["token"]["secret"].as_str().unwrap())
        .await;

    let me: Value = w
        .client
        .get(w.url("/me"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["member_id"], json!(w.a.member.0), "{me}");
    // The grant lent workspace:read only, whatever the delegate holds itself.
    let post = w
        .client
        .post(w.url(&format!("/threads/{}/messages", w.a.thread.0)))
        .header(header::COOKIE, &cookie)
        .json(&json!({ "body": "not lent" }))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), StatusCode::FORBIDDEN);

    let session = w
        .store
        .list_audit_for_workspace(w.a.ws, 50)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.action == "session.from_token")
        .expect("audited");
    assert_eq!(session.actor_id, Some(delegate), "the delegate acted");
    assert_eq!(session.metadata["grant_id"], grant["id"]);

    let revoked = w
        .client
        .delete(w.url(&format!(
            "/workspaces/{}/delegation-grants/{}",
            w.a.ws.0,
            grant["id"].as_str().unwrap()
        )))
        .bearer_auth(&w.a.admin)
        .send()
        .await
        .unwrap();
    assert!(revoked.status().is_success(), "{}", revoked.status());
    assert_eq!(w.get(&cookie, "/me").await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn only_a_bearer_is_exchanged_and_sign_out_ends_the_session() {
    let w = world(Sessions::Standalone).await;
    let cookie = w.exchange(&w.a.admin).await;

    // A session cannot make another (and so outlive its own lifetime).
    let chained = w
        .client
        .post(w.url("/auth/session/from-token"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(chained.status(), StatusCode::UNAUTHORIZED);

    let out = w
        .client
        .post(w.url("/auth/logout"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        out.headers().get(header::LOCATION).unwrap(),
        "/ui/",
        "a token's session is not sent to an identity provider"
    );
    assert_eq!(w.get(&cookie, "/me").await, StatusCode::UNAUTHORIZED);
    // The token itself is untouched.
    let bearer = w
        .client
        .get(w.url("/me"))
        .bearer_auth(&w.a.admin)
        .send()
        .await
        .unwrap();
    assert_eq!(bearer.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_tokens_session_cannot_mint_the_first_admin_token() {
    let w = world(Sessions::Oidc).await;
    let (secret, _) = w
        .mint(&w.a, w.a.member, &[capability::WORKSPACE_READ])
        .await;
    // No token:admin holder remains, so an OIDC session could mint one.
    w.store.revoke_api_token(w.a.admin_id).await.unwrap();
    let cookie = w.exchange(&secret).await;
    let mint = w
        .client
        .post(w.url("/auth/session/mint"))
        .header(header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(mint.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn without_a_session_secret_there_is_no_exchange() {
    let w = world(Sessions::None).await;
    let resp = w
        .client
        .post(w.url("/auth/session/from-token"))
        .bearer_auth(&w.a.admin)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
