//! Agent self-reported status over REST: `PUT /threads/:id/status` declares,
//! `GET` reads it back, each declaration lands in the event log as
//! `StatusDeclared`, a newer declaration supersedes the older one, and the
//! system-computed `stalled` is not something an agent can declare. Auth is
//! enabled with real tokens, and a second workspace can neither declare on
//! nor read the first one's thread.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    DeclaredStatus, Event, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread,
    NewWorkspace, Workspace,
};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

async fn workspace_with_agent(store: &Arc<dyn Store>, name: &str) -> (Workspace, MemberId, String) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: agent.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some(name.into()),
            capabilities: vec!["workspace:read".into(), "thread:transition".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws, agent.id, format!("Bearer {}", secret.as_str()))
}

#[tokio::test]
async fn a_declared_status_is_read_back_logged_and_superseded_and_stays_in_its_workspace() {
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

    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let (ws, agent, auth) = workspace_with_agent(&store, "a").await;
    let (_other_ws, _other_agent, other_auth) = workspace_with_agent(&store, "b").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parser".into()),
        })
        .await
        .unwrap();
    // Status is declared by the claim holder or the thread owner: make the
    // agent the owner so its declarations are accepted.
    store
        .set_thread_owner(thread.id, Some(agent))
        .await
        .unwrap();
    let url = format!("{base}/threads/{}/status", thread.id.0);

    // Nothing declared yet.
    let none = client
        .get(&url)
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), StatusCode::NOT_FOUND);

    // Declare, then read it back.
    let put = client
        .put(&url)
        .header("Authorization", &auth)
        .json(&json!({ "status": "working", "note": "Writing the parser." }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let declared: serde_json::Value = put.json().await.unwrap();
    assert_eq!(declared["status"], "working");
    assert_eq!(declared["note"], "Writing the parser.");
    assert_eq!(declared["declared_by"], agent.0.to_string());

    let got: serde_json::Value = client
        .get(&url)
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["status"], "working");

    // The declaration is in the workspace's event log as StatusDeclared.
    let declared_events: Vec<(DeclaredStatus, String, MemberId)> = store
        .list_events_after(ws.id, 0, 500)
        .await
        .unwrap()
        .iter()
        .filter_map(|e| match e.opened_event().ok()? {
            Event::StatusDeclared {
                thread_id,
                status,
                note,
                declared_by,
                ..
            } if thread_id == thread.id => Some((status, note, declared_by)),
            _ => None,
        })
        .collect();
    assert_eq!(
        declared_events,
        vec![(
            DeclaredStatus::Working,
            "Writing the parser.".to_string(),
            agent
        )],
        "one StatusDeclared event, carrying the status, note and declarer"
    );

    // A newer declaration supersedes the older one.
    let again = client
        .put(&url)
        .header("Authorization", &auth)
        .json(&json!({ "status": "needs_input", "note": "Which date format?" }))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::OK);
    let now: serde_json::Value = client
        .get(&url)
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(now["status"], "needs_input");
    assert_eq!(now["note"], "Which date format?");

    // `stalled` is system-computed, so an agent cannot declare it; a blank
    // or multi-line note is refused.
    for bad in [
        json!({ "status": "stalled", "note": "Nothing is happening." }),
        json!({ "status": "working", "note": "   " }),
        json!({ "status": "working", "note": "One.\nTwo." }),
    ] {
        let res = client
            .put(&url)
            .header("Authorization", &auth)
            .json(&bad)
            .send()
            .await
            .unwrap();
        assert!(
            res.status().is_client_error(),
            "{bad} should be refused, got {}",
            res.status()
        );
    }

    // Another workspace can neither declare on this thread nor read its status.
    let cross_put = client
        .put(&url)
        .header("Authorization", &other_auth)
        .json(&json!({ "status": "done", "note": "Not mine to say." }))
        .send()
        .await
        .unwrap();
    assert!(
        matches!(
            cross_put.status(),
            StatusCode::NOT_FOUND | StatusCode::FORBIDDEN
        ),
        "a cross-workspace declare must be refused, got {}",
        cross_put.status()
    );
    let cross_get = client
        .get(&url)
        .header("Authorization", &other_auth)
        .send()
        .await
        .unwrap();
    assert!(
        matches!(
            cross_get.status(),
            StatusCode::NOT_FOUND | StatusCode::FORBIDDEN
        ),
        "a cross-workspace read must be refused, got {}",
        cross_get.status()
    );
    let still: serde_json::Value = client
        .get(&url)
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        still["status"], "needs_input",
        "the refused cross-workspace declare changed nothing"
    );

    server.abort();
}

/// Only the claim holder or the thread owner may declare: a third member
/// with thread access but neither role is refused, and the refusal changes
/// nothing.
#[tokio::test]
async fn a_stranger_cannot_declare_status_on_anothers_thread() {
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

    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let (ws, owner, owner_auth) = workspace_with_agent(&store, "owner-ws").await;
    // A second member in the same workspace, with the same capabilities.
    let stranger = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "stranger".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let stranger_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: stranger.id,
            app_installation_id: None,
            token_hash: hash_secret(stranger_secret.as_str()),
            label: Some("stranger".into()),
            capabilities: vec!["workspace:read".into(), "thread:transition".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let stranger_auth = format!("Bearer {}", stranger_secret.as_str());

    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parser".into()),
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(owner))
        .await
        .unwrap();
    let url = format!("{base}/threads/{}/status", thread.id.0);

    // The stranger holds thread:transition and thread access, but is neither
    // the claim holder nor the owner: refused.
    let refused = client
        .put(&url)
        .header("Authorization", &stranger_auth)
        .json(&json!({ "status": "working", "note": "Hijacking the status." }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        StatusCode::CONFLICT,
        "a non-holder non-owner declare must be refused"
    );

    // Nothing was written: the owner reads no declaration.
    let none = client
        .get(&url)
        .header("Authorization", &owner_auth)
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), StatusCode::NOT_FOUND);

    // The owner can still declare afterwards.
    let ok = client
        .put(&url)
        .header("Authorization", &owner_auth)
        .json(&json!({ "status": "working", "note": "Writing the parser." }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    server.abort();
}

/// A human response clears the declaration: the agent's status describes what
/// it was doing, and the human's reply supersedes it.
#[tokio::test]
async fn a_human_response_clears_the_declared_status() {
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

    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let (ws, agent, agent_auth) = workspace_with_agent(&store, "clear-ws").await;
    // A human member in the same workspace, able to post.
    let human = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "human".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let human_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: human.id,
            app_installation_id: None,
            token_hash: hash_secret(human_secret.as_str()),
            label: Some("human".into()),
            capabilities: vec!["workspace:read".into(), "message:post".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let human_auth = format!("Bearer {}", human_secret.as_str());

    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parser".into()),
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(agent))
        .await
        .unwrap();
    let status_url = format!("{base}/threads/{}/status", thread.id.0);

    // The agent declares.
    let put = client
        .put(&status_url)
        .header("Authorization", &agent_auth)
        .json(&json!({ "status": "needs_input", "note": "Which date format?" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

    // The human replies on the thread.
    let post = client
        .post(format!("{base}/threads/{}/messages", thread.id.0))
        .header("Authorization", &human_auth)
        .json(&json!({ "body": "Use ISO 8601." }))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), StatusCode::CREATED);

    // The declaration is gone.
    let gone = client
        .get(&status_url)
        .header("Authorization", &agent_auth)
        .send()
        .await
        .unwrap();
    assert_eq!(
        gone.status(),
        StatusCode::NOT_FOUND,
        "a human response should clear the declared status"
    );

    server.abort();
}
