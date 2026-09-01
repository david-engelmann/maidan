//! Cluster 310: the GitHub projector ingress. A correctly-signed `ping` is ACKed;
//! the route is `404` when the projector is not configured and `401` on a bad
//! `X-Hub-Signature-256`.

use std::net::SocketAddr;
use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{github::GithubConfig, router, webhooks::sign_payload, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewGithubIssueLink, NewMember, NewThread, NewWorkspace,
};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

const SECRET: &str = "gh-webhook-secret";

async fn spawn(
    with_github: bool,
) -> (
    SocketAddr,
    reqwest::Client,
    Arc<dyn Store>,
    tokio::task::JoinHandle<()>,
) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    std::mem::forget(dir);
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    if with_github {
        state.attach_github(Arc::new(GithubConfig {
            webhook_secret: SECRET.into(),
            api_token: None,
        }));
    }
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store, handle)
}

#[tokio::test]
async fn github_ping_is_acked() {
    let (addr, client, _store, server) = spawn(true).await;
    let body = r#"{"zen":"Keep it simple."}"#;
    let sig = sign_payload(SECRET, body);
    let resp = client
        .post(format!("http://{addr}/integrations/github/events"))
        .header("x-github-event", "ping")
        .header("x-hub-signature-256", &sig)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    server.abort();
}

#[tokio::test]
async fn github_ingress_is_404_when_not_configured() {
    let (addr, client, _store, server) = spawn(false).await;
    let resp = client
        .post(format!("http://{addr}/integrations/github/events"))
        .header("x-github-event", "ping")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    server.abort();
}

#[tokio::test]
async fn github_bad_signature_is_401() {
    let (addr, client, _store, server) = spawn(true).await;
    let resp = client
        .post(format!("http://{addr}/integrations/github/events"))
        .header("x-github-event", "ping")
        .header("x-hub-signature-256", "sha256=deadbeef")
        .header("content-type", "application/json")
        .body(r#"{"zen":"nope"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    server.abort();
}

#[tokio::test]
async fn github_issue_comment_in_a_linked_issue_posts_a_maidan_message() {
    let (addr, client, store, server) = spawn(true).await;
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "ghbot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "eng".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("issue-42".into()),
        })
        .await
        .unwrap();
    store
        .link_github_issue(NewGithubIssueLink {
            repo: "o/r".into(),
            issue_number: 42,
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: bot.id,
        })
        .await
        .unwrap();

    // A signed issue_comment.created for o/r#42 is projected into the thread.
    let body = r#"{"action":"created","repository":{"full_name":"o/r"},"issue":{"number":42},"comment":{"body":"needs review","user":{"login":"octocat","type":"User"}}}"#;
    let sig = sign_payload(SECRET, body);
    let resp = client
        .post(format!("http://{addr}/integrations/github/events"))
        .header("x-github-event", "issue_comment")
        .header("x-hub-signature-256", &sig)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let msgs = store.list_messages(thread.id, 50).await.unwrap();
    assert!(
        msgs.iter().any(|m| m.body == "octocat: needs review"),
        "the GitHub comment was projected into the Maidan thread"
    );

    // A Bot comment (our own egress echo) is not re-ingested — no loop.
    let bot_body = r#"{"action":"created","repository":{"full_name":"o/r"},"issue":{"number":42},"comment":{"body":"echo","user":{"login":"maidan[bot]","type":"Bot"}}}"#;
    let sig2 = sign_payload(SECRET, bot_body);
    client
        .post(format!("http://{addr}/integrations/github/events"))
        .header("x-github-event", "issue_comment")
        .header("x-hub-signature-256", &sig2)
        .header("content-type", "application/json")
        .body(bot_body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(
        store.list_messages(thread.id, 50).await.unwrap().len(),
        1,
        "a Bot comment is not re-projected"
    );

    server.abort();
}
