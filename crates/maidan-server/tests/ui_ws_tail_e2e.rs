//! Cluster 93: /ui WS subscribe with session cookie, filter presets, resume reconnect.

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
    router, subscribe_resume, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{NewMember, NewWorkspace, WorkspaceId};
use reqwest::redirect::Policy;
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
            name: "ui-ws".into(),
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
async fn ui_ws_subscribe_accepts_session_cookie_and_resume_token() {
    let h = spawn_oidc().await;
    let cookie = login_session(&h).await;
    let wid = h.workspace_id.0;
    let ws_url = format!("ws://{}/ws/subscribe", h.addr);

    let mut req = ws_url.clone().into_client_request().expect("ws request");
    req.headers_mut()
        .insert("Cookie", cookie.parse().expect("cookie header"));

    let (mut ws, _) = connect_async(req).await.expect("ws connect");
    let frame = json!({
        "filter": { "workspace_id": wid, "kinds": ["message_posted"] },
        "after_id": 0
    });
    ws.send(Message::Text(frame.to_string()))
        .await
        .expect("subscribe send");

    let mut resume_token = None;
    for _ in 0..20 {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("ws timeout")
            .expect("ws stream")
            .expect("ws frame");
        if let Message::Text(text) = msg {
            let v: serde_json::Value = serde_json::from_str(&text).expect("json");
            if v.get("type").and_then(|t| t.as_str()) == Some("subscribe_ack") {
                resume_token = v
                    .get("resume_token")
                    .and_then(|t| t.as_str())
                    .map(String::from);
                break;
            }
        }
    }
    let resume_token = resume_token.expect("subscribe_ack with resume_token");

    ws.close(None).await.ok();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut req2 = ws_url.clone().into_client_request().expect("ws request 2");
    req2.headers_mut()
        .insert("Cookie", cookie.parse().expect("cookie"));
    let (mut ws2, _) = connect_async(req2).await.expect("reconnect");
    let reconnect = json!({ "resume_token": resume_token });
    ws2.send(Message::Text(reconnect.to_string()))
        .await
        .expect("resume send");
    let msg = tokio::time::timeout(Duration::from_secs(5), ws2.next())
        .await
        .expect("ack timeout")
        .expect("stream")
        .expect("frame");
    if let Message::Text(text) = msg {
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(
            v.get("type").and_then(|t| t.as_str()),
            Some("subscribe_ack")
        );
    }

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
    assert!(html.contains("ws-preset"));
    assert!(html.contains("ws-auto-reconnect"));

    h.server.abort();
}
