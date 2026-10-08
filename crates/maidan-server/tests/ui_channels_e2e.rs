//! Channel browser via `/ui/api` with session cookie (no bearer), including
//! the signed-session write-to-live-WebSocket collaboration loop.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, NewMember, NewWorkspace, ThreadId, WorkspaceId};
use reqwest::{redirect::Policy, StatusCode};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

const TEST_SESSION_SECRET: &[u8] = b"test-session-secret-32-bytes-min!";

struct Harness {
    addr: SocketAddr,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    workspace_id: WorkspaceId,
    store: Arc<dyn Store>,
}

async fn spawn_oidc() -> Harness {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "ui-channels".into(),
        })
        .await
        .expect("workspace");
    store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "alice".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .expect("member");

    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
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
        session_secret: Arc::from(TEST_SESSION_SECRET),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");

    Harness {
        addr,
        client,
        server,
        workspace_id: workspace.id,
        store,
    }
}

async fn login_session(h: &Harness) -> String {
    let base = format!("http://{}", h.addr);
    let wid = h.workspace_id.0;
    let login = h
        .client
        .get(format!("{base}/auth/oidc/login?workspace_id={wid}"))
        .send()
        .await
        .expect("login");
    assert_eq!(login.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = login
        .headers()
        .get(reqwest::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let callback = h
        .client
        .get(format!("{base}{location}"))
        .send()
        .await
        .expect("callback");
    assert_eq!(callback.status(), StatusCode::TEMPORARY_REDIRECT);
    callback
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .find_map(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .next()
                .filter(|p| p.starts_with("maidan_session="))
        })
        .expect("session cookie")
        .to_string()
}

#[tokio::test]
async fn ui_shell_exposes_channel_browser_markers() {
    let h = spawn_oidc().await;
    let html = h
        .client
        .get(format!("http://{}/ui/", h.addr))
        .send()
        .await
        .expect("ui")
        .text()
        .await
        .expect("html");
    assert!(html.contains("/ui/static/main.js"));
    // The write helpers moved out of the shell into the module it loads.
    let api_js = h
        .client
        .get(format!("http://{}/ui/static/api.js", h.addr))
        .send()
        .await
        .expect("api.js")
        .text()
        .await
        .expect("api.js body");
    assert!(api_js.contains("function apiWritePath"));
    assert!(api_js.contains("function requireAuthForWrite"));
    h.server.abort();
}

#[tokio::test]
async fn ui_api_signed_session_hero_loop_reaches_the_live_websocket() {
    let h = spawn_oidc().await;
    let base = format!("http://{}", h.addr);
    let wid = h.workspace_id.0;
    let cookie = login_session(&h).await;

    let session: serde_json::Value = h
        .client
        .get(format!("{base}/auth/session"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("session")
        .json()
        .await
        .expect("session json");
    let member_id = session["member_id"].as_str().expect("member_id");

    let mut ws_request = format!("ws://{}/ws/subscribe", h.addr)
        .into_client_request()
        .expect("ws request");
    ws_request
        .headers_mut()
        .insert("Cookie", cookie.parse().expect("cookie header"));
    let (mut ws, _) = connect_async(ws_request).await.expect("ws connect");
    ws.send(Message::Text(
        json!({
            "filter": {
                "workspace_id": wid,
                "kinds": ["message_posted"]
            }
        })
        .to_string(),
    ))
    .await
    .expect("subscribe send");

    let subscribed = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let frame: serde_json::Value =
                        serde_json::from_str(&text).expect("subscribe frame json");
                    if frame["type"] == "subscribe_ack" {
                        return;
                    }
                }
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                other => panic!("unexpected frame before subscribe_ack: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), subscribed)
        .await
        .expect("subscribe_ack timeout");

    let channel: serde_json::Value = h
        .client
        .post(format!("{base}/ui/api/workspaces/{wid}/channels"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({"name": "general", "private": false}))
        .send()
        .await
        .expect("create channel")
        .error_for_status()
        .expect("channel status")
        .json()
        .await
        .expect("channel json");
    let channel_id = channel["id"].as_str().expect("channel id");

    let channels: Vec<serde_json::Value> = h
        .client
        .get(format!("{base}/ui/api/workspaces/{wid}/channels"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("list channels")
        .json()
        .await
        .expect("channels json");
    assert_eq!(channels.len(), 1);

    let thread: serde_json::Value = h
        .client
        .post(format!("{base}/ui/api/channels/{channel_id}/threads"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({"title": "standup"}))
        .send()
        .await
        .expect("create thread")
        .error_for_status()
        .expect("thread status")
        .json()
        .await
        .expect("thread json");
    let thread_id = thread["id"].as_str().expect("thread id");

    let threads: Vec<serde_json::Value> = h
        .client
        .get(format!("{base}/ui/api/channels/{channel_id}/threads"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("list threads")
        .json()
        .await
        .expect("threads json");
    assert_eq!(threads.len(), 1);

    let msg: serde_json::Value = h
        .client
        .post(format!("{base}/ui/api/threads/{thread_id}/messages"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({
            "body": "posted from ui session api"
        }))
        .send()
        .await
        .expect("post message")
        .error_for_status()
        .expect("message status")
        .json()
        .await
        .expect("message json");
    assert_eq!(msg["body"], "posted from ui session api");
    let message_id = msg["id"].as_str().expect("message id");

    let observed = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let frame: serde_json::Value =
                        serde_json::from_str(&text).expect("event frame json");
                    if frame["kind"] == "message_posted" {
                        return frame;
                    }
                }
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                other => panic!("unexpected frame before message_posted: {other:?}"),
            }
        }
    };
    let observed = tokio::time::timeout(Duration::from_secs(5), observed)
        .await
        .expect("message_posted timeout");
    assert_eq!(observed["workspace_id"], wid.to_string());
    assert_eq!(observed["thread_id"], thread_id);
    assert_eq!(observed["message"]["id"], message_id);
    assert_eq!(observed["message"]["author_id"], member_id);
    assert_eq!(observed["message"]["body"], "posted from ui session api");
    assert!(
        observed["log_id"].as_i64().is_some_and(|id| id > 0),
        "live event must carry its durable log id: {observed}"
    );

    let messages: Vec<serde_json::Value> = h
        .client
        .get(format!(
            "{base}/ui/api/threads/{thread_id}/messages?limit=10"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("list messages")
        .json()
        .await
        .expect("messages json");
    assert_eq!(messages.len(), 1);

    let wrong_author = h
        .client
        .post(format!("{base}/ui/api/threads/{thread_id}/messages"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({
            "author_id": "00000000-0000-0000-0000-000000000099",
            "body": "spoof"
        }))
        .send()
        .await
        .expect("spoof post");
    assert_eq!(wrong_author.status(), StatusCode::BAD_REQUEST);

    ws.close(None).await.ok();
    h.server.abort();
}

/// Acting identity fields are rejected at the boundary; the authenticated
/// member is used for both message authorship and reactions.
#[tokio::test]
async fn session_identity_is_the_only_message_and_reaction_actor() {
    let h = spawn_oidc().await;
    let base = format!("http://{}", h.addr);
    let wid = h.workspace_id.0;
    let cookie = login_session(&h).await;
    let session: serde_json::Value = h
        .client
        .get(format!("{base}/auth/session"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("session")
        .json()
        .await
        .expect("session json");
    let member_id = session["member_id"]
        .as_str()
        .expect("member_id")
        .to_string();

    let ch: serde_json::Value = h
        .client
        .post(format!("{base}/ui/api/workspaces/{wid}/channels"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "name": "react" }))
        .send()
        .await
        .expect("channel")
        .json()
        .await
        .expect("channel json");
    let cid = ch["id"].as_str().expect("cid");
    let th: serde_json::Value = h
        .client
        .post(format!("{base}/ui/api/channels/{cid}/threads"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "title": "t" }))
        .send()
        .await
        .expect("thread")
        .json()
        .await
        .expect("thread json");
    let tid = th["id"].as_str().expect("tid");
    let msg: serde_json::Value = h
        .client
        .post(format!("{base}/ui/api/threads/{tid}/messages"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "body": "hi" }))
        .send()
        .await
        .expect("message")
        .json()
        .await
        .expect("message json");
    let mid = msg["id"].as_str().expect("mid");

    assert_eq!(msg["author_id"], member_id);

    // The removed voter field is an unknown input, even when it names self.
    let spoof = h
        .client
        .post(format!("{base}/ui/api/messages/{mid}/reactions"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "member_id": "00000000-0000-0000-0000-000000000099", "emoji": "👍" }))
        .send()
        .await
        .expect("spoof reaction");
    assert_eq!(spoof.status(), StatusCode::BAD_REQUEST);

    // With no acting identity in the request, the session member is recorded.
    let ok = h
        .client
        .post(format!("{base}/ui/api/messages/{mid}/reactions"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "emoji": "👍" }))
        .send()
        .await
        .expect("authenticated reaction");
    assert!(ok.status().is_success(), "a session may react as itself");

    h.server.abort();
}

/// The legacy `/members/:id/mentions` + `/inbox` routes live ONLY on the
/// bearer-only `protected` router, so a browser session cannot reach them — the
/// audit's "a session can read another member's inbox" was a false positive on
/// reachability. This documents that truth:
/// a session cookie with no bearer gets `403` and a sentence, never another member's data.
/// The handlers also require self or explicit delegated personal-state access,
/// guarding any future `/ui/api` session mount.
#[tokio::test]
async fn legacy_inbox_and_mentions_are_bearer_only_not_session_reachable() {
    let h = spawn_oidc().await;
    let base = format!("http://{}", h.addr);
    let cookie = login_session(&h).await;
    let session: serde_json::Value = h
        .client
        .get(format!("{base}/auth/session"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("session")
        .json()
        .await
        .expect("session json");
    // Even presenting its OWN member id, a session can't use these bearer-only routes.
    let my_id = session["member_id"].as_str().expect("member_id");
    for path in [
        format!("{base}/members/{my_id}/inbox"),
        format!("{base}/members/{my_id}/mentions"),
    ] {
        let resp = h
            .client
            .get(&path)
            .header(reqwest::header::COOKIE, &cookie)
            .send()
            .await
            .expect("session on bearer-only route");
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "a session cookie does not authenticate on the bearer-only route {path}"
        );
        let body: serde_json::Value = resp.json().await.expect("problem");
        let detail = body["detail"].as_str().unwrap_or("");
        assert!(
            detail.contains("A signed-in session cannot call it."),
            "bearer-only route should answer a session with a sentence, got {body}"
        );
    }

    h.server.abort();
}

/// An OIDC session edits, uploads, and reads through `/ui/api`, and can move a
/// thread. A route that stays bearer-only answers that session with a sentence.
/// An approval from the thread's owner is recorded and does not count.
#[tokio::test]
async fn oidc_session_edits_uploads_and_transitions_through_the_proxy() {
    let h = spawn_oidc().await;
    let base = format!("http://{}", h.addr);
    let wid = h.workspace_id.0;
    let cookie = login_session(&h).await;
    let session: serde_json::Value = h
        .client
        .get(format!("{base}/auth/session"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("session")
        .json()
        .await
        .expect("session json");
    let member_id = session["member_id"].as_str().expect("member_id");

    let workspace: serde_json::Value = h
        .client
        .get(format!("{base}/ui/api/workspaces/{wid}"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("workspace")
        .error_for_status()
        .expect("workspace status")
        .json()
        .await
        .expect("workspace json");
    assert_eq!(workspace["name"], "ui-channels");

    let channel: serde_json::Value = h
        .client
        .post(format!("{base}/ui/api/workspaces/{wid}/channels"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({"name": "writes", "private": false}))
        .send()
        .await
        .expect("channel")
        .error_for_status()
        .expect("channel status")
        .json()
        .await
        .expect("channel json");
    let channel_id = channel["id"].as_str().expect("channel id");
    let thread: serde_json::Value = h
        .client
        .post(format!("{base}/ui/api/channels/{channel_id}/threads"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({"title": "edit me"}))
        .send()
        .await
        .expect("thread")
        .error_for_status()
        .expect("thread status")
        .json()
        .await
        .expect("thread json");
    let thread_id = thread["id"].as_str().expect("thread id");
    let posted: serde_json::Value = h
        .client
        .post(format!("{base}/ui/api/threads/{thread_id}/messages"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({"body": "first"}))
        .send()
        .await
        .expect("post")
        .error_for_status()
        .expect("post status")
        .json()
        .await
        .expect("post json");
    let message_id = posted["id"].as_str().expect("message id");

    let edited: serde_json::Value = h
        .client
        .patch(format!("{base}/ui/api/messages/{message_id}"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({"body": "edited by the session"}))
        .send()
        .await
        .expect("edit")
        .error_for_status()
        .expect("edit status")
        .json()
        .await
        .expect("edit json");
    assert_eq!(edited["body"], "edited by the session");

    let artifact = h
        .client
        .post(format!(
            "{base}/ui/api/artifacts?kind=attachment&filename=note.txt"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .header(reqwest::header::CONTENT_TYPE, "text/plain")
        .body("pasted bytes")
        .send()
        .await
        .expect("upload");
    assert_eq!(artifact.status(), StatusCode::CREATED, "upload");
    let artifact: serde_json::Value = artifact.json().await.expect("artifact json");
    assert!(artifact["sha256"].as_str().is_some_and(|s| !s.is_empty()));

    let got = h
        .client
        .get(format!("{base}/ui/api/threads/{thread_id}"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("get thread")
        .error_for_status()
        .expect("get thread status")
        .json::<serde_json::Value>()
        .await
        .expect("thread json");
    assert_eq!(got["id"], thread_id);

    let status = h
        .client
        .get(format!("{base}/ui/api/threads/{thread_id}/review-status"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("review status")
        .error_for_status()
        .expect("review status code")
        .json::<serde_json::Value>()
        .await
        .expect("review status json");
    assert_eq!(status["approvals"], json!(0));

    let started = h
        .client
        .post(format!("{base}/ui/api/threads/{thread_id}"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({"action": "start_review"}))
        .send()
        .await
        .expect("start review");
    assert!(
        !matches!(
            started.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ),
        "a session with thread:transition was refused: {} {}",
        started.status(),
        started.text().await.unwrap_or_default()
    );

    let thread_uuid = thread_id.parse().expect("thread uuid");
    let member_uuid = member_id.parse().expect("member uuid");
    h.store
        .set_thread_owner(ThreadId(thread_uuid), Some(MemberId(member_uuid)))
        .await
        .expect("owner");
    h.store
        .set_review_requirement(ThreadId(thread_uuid), 1)
        .await
        .expect("requirement");
    let root = h
        .store
        .latest_review_packet(ThreadId(thread_uuid))
        .await
        .expect("packet")
        .expect("handed to review")
        .evidence_root;
    let review = h
        .client
        .post(format!("{base}/ui/api/threads/{thread_id}/reviews"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({"decision": "approve", "evidence_root": root}))
        .send()
        .await
        .expect("review")
        .error_for_status()
        .expect("review status");
    assert!(review.status().is_success());
    let after: serde_json::Value = h
        .client
        .get(format!("{base}/ui/api/threads/{thread_id}/review-status"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("status after")
        .error_for_status()
        .expect("status after code")
        .json()
        .await
        .expect("status after json");
    assert_eq!(
        after["approvals"],
        json!(0),
        "the owner's own approval is borrowed authority and does not count: {after}"
    );

    let purge = h
        .client
        .post(format!("{base}/workspaces/{wid}/purge"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({}))
        .send()
        .await
        .expect("purge");
    assert_eq!(purge.status(), StatusCode::FORBIDDEN);
    let problem: serde_json::Value = purge.json().await.expect("purge problem");
    assert!(
        problem["detail"]
            .as_str()
            .unwrap_or("")
            .contains("A signed-in session cannot call it."),
        "{problem}"
    );

    h.server.abort();
}
