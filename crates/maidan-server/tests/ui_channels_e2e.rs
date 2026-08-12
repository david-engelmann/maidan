//! Cluster 92: channel browser via `/ui/api` with session cookie (no bearer).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{NewMember, NewWorkspace, WorkspaceId};
use reqwest::{redirect::Policy, StatusCode};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

const TEST_SESSION_SECRET: &[u8] = b"test-session-secret-32-bytes-min!";

struct Harness {
    addr: SocketAddr,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    workspace_id: WorkspaceId,
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
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
        store,
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
    assert!(html.contains(r#"data-ui-version="7""#));
    assert!(html.contains("apiWritePath"));
    assert!(html.contains("requireAuthForWrite"));
    h.server.abort();
}

#[tokio::test]
async fn ui_api_session_posts_channel_thread_and_message_without_bearer() {
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
            "author_id": member_id,
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
    assert_eq!(wrong_author.status(), StatusCode::FORBIDDEN);

    h.server.abort();
}

/// Cluster 202: the anti-spoofing guard is wired on a *newly-guarded* surface
/// (reactions), not only on `post_message` — a session caller cannot react as
/// another member, but may react as itself.
#[tokio::test]
async fn session_cannot_react_as_another_member() {
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
        .json(&json!({ "author_id": member_id, "body": "hi" }))
        .send()
        .await
        .expect("message")
        .json()
        .await
        .expect("message json");
    let mid = msg["id"].as_str().expect("mid");

    // React as ANOTHER member → 403 (the newly-guarded add_reaction surface).
    let spoof = h
        .client
        .post(format!("{base}/ui/api/messages/{mid}/reactions"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "member_id": "00000000-0000-0000-0000-000000000099", "emoji": "👍" }))
        .send()
        .await
        .expect("spoof reaction");
    assert_eq!(spoof.status(), StatusCode::FORBIDDEN);

    // React as itself → allowed.
    let ok = h
        .client
        .post(format!("{base}/ui/api/messages/{mid}/reactions"))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&json!({ "member_id": member_id, "emoji": "👍" }))
        .send()
        .await
        .expect("self reaction");
    assert!(ok.status().is_success(), "a session may react as itself");

    h.server.abort();
}
