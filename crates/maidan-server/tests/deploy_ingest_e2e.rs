//! Deploy push-ingest over HTTP: the route is a 404 without a hook, a 401 on
//! a bad signature and a 400 on a malformed event; a signed deploy of a
//! merged pull request's commit posts the notice on its thread, once.

use std::net::SocketAddr;
use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::deploy_ingest::{DeployHookConfig, SIGNATURE_HEADER};
use maidan_server::{router, webhooks::sign_payload, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChangeRound, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId,
    ThreadNoticeKind,
};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

const SECRET: &str = "deploy-hook-secret";
const URL_PATH: &str = "/integrations/deploys/events";

async fn spawn(with_hook: bool) -> (SocketAddr, Arc<dyn Store>, tokio::task::JoinHandle<()>) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    std::mem::forget(dir);
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    if with_hook {
        let hook = DeployHookConfig::from_secret(Some(SECRET.into())).unwrap();
        state.attach_deploy_hook(Arc::new(hook));
    }
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, store, handle)
}

/// A thread owned by a member, tracking `example/app#7` merged at `head`.
async fn merged_thread(store: &dyn Store, head: &str) -> ThreadId {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "dev".into(),
            display_name: None,
            kind: MemberKind::Human,
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
            title: Some("task".into()),
            description: None,
            created_by: Some(member.id),
        })
        .await
        .unwrap()
        .id;
    store
        .set_thread_owner(thread, Some(member.id))
        .await
        .unwrap();
    store
        .record_change_round(NewChangeRound {
            workspace_id: ws.id,
            thread_id: thread,
            repo: "example/app".into(),
            pull_number: 7,
            commit_sha: head.into(),
            base_sha: "b".repeat(40),
            base_behind: None,
            diff_lines: 1,
            paths: vec!["src/lib.rs".into()],
            base_branch: Some("main".into()),
            rebase: false,
            previous_head: None,
            linear_issue: None,
        })
        .await
        .unwrap();
    let at = chrono::Utc::now();
    store
        .close_change_pull(ws.id, "example/app", 7, Some(at), Some(head), at)
        .await
        .unwrap();
    thread
}

async fn send(addr: SocketAddr, body: &str, signature: Option<String>) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .post(format!("http://{addr}{URL_PATH}"))
        .header("content-type", "application/json")
        .body(body.to_string());
    if let Some(signature) = signature {
        request = request.header(SIGNATURE_HEADER, signature);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn the_deploy_route_is_a_404_until_a_hook_is_set() {
    let (addr, _store, server) = spawn(false).await;
    let body = "{}";
    let resp = send(addr, body, Some(sign_payload(SECRET, body))).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    server.abort();
}

#[tokio::test]
async fn a_deploy_event_with_a_bad_or_missing_signature_is_refused() {
    let (addr, _store, server) = spawn(true).await;
    let body = r#"{"repo":"example/app","sha":"x","environment":"p","status":"success"}"#;
    assert_eq!(
        send(addr, body, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let wrong = sign_payload("another-secret", body);
    assert_eq!(
        send(addr, body, Some(wrong)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    server.abort();
}

#[tokio::test]
async fn a_signed_but_malformed_deploy_event_is_a_400() {
    let (addr, _store, server) = spawn(true).await;
    for body in [
        "not json",
        r#"{"repo":"example/app","sha":"abc1234","environment":"production","status":"success"}"#,
        r#"{"repo":"example/app","sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","environment":"production","status":"success","extra":1}"#,
    ] {
        let resp = send(addr, body, Some(sign_payload(SECRET, body))).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    server.abort();
}

#[tokio::test]
async fn a_signed_deploy_of_a_merged_commit_is_said_once_on_its_thread() {
    let (addr, store, server) = spawn(true).await;
    let head = "a".repeat(40);
    let thread = merged_thread(store.as_ref(), &head).await;
    let event = |status: &str| {
        format!(
            r#"{{"repo":"Example/App","sha":"{head}","environment":"production","status":"{status}","url":"https://deploys.example/42"}}"#
        )
    };
    let noticed = |resp: serde_json::Value| resp["noticed"].as_u64();

    let running = event("in_progress");
    let resp = send(addr, &running, Some(sign_payload(SECRET, &running))).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        noticed(resp.json().await.unwrap()),
        Some(0),
        "a running deploy is not news yet"
    );

    let done = event("success");
    for expected in [1, 0] {
        let resp = send(addr, &done, Some(sign_payload(SECRET, &done))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(noticed(resp.json().await.unwrap()), Some(expected));
    }

    let notices = store.list_thread_notices(thread, 10).await.unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, ThreadNoticeKind::Deployed);
    let messages = store.list_messages(thread, 10).await.unwrap();
    assert!(
        messages.iter().any(|m| m
            .body
            .starts_with("example/app#7 is deployed to production")),
        "the thread's owner says it: {:?}",
        messages.iter().map(|m| &m.body).collect::<Vec<_>>()
    );
    server.abort();
}
