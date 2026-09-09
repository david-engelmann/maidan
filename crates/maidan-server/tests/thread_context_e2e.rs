//! `GET /threads/:id/context` packs thread data for agent prompts.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::{ArtifactStore, LocalFsStore};
use maidan_auth::{hash_secret, TokenSecret};
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EditMessage, MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewReference,
    NewThread, NewWorkspace, RefSide,
};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn thread_context_includes_messages_refs_artifacts_and_fsm() {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(
        store.clone(),
        artifacts.clone(),
        bus,
        search,
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "ctx-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
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
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("context thread".into()),
        })
        .await
        .unwrap();

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("ctx".into()),
            capabilities: vec!["workspace:read".into(), "thread:transition".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());

    store
        .transition_thread(thread.id, member.id, maidan_fsm::ThreadAction::StartReview)
        .await
        .unwrap();

    let payload = b"context artifact bytes";
    let sha = maidan_artifacts::Sha256::compute(payload);
    artifacts
        .put(bytes::Bytes::from_static(payload))
        .await
        .unwrap();
    let artifact = store
        .upsert_artifact(maidan_types::NewArtifact {
            sha256: sha.to_hex(),
            size_bytes: payload.len() as i64,
            mime_type: Some("text/plain".into()),
            kind: maidan_types::ArtifactKind::Attachment,
            uploaded_by: Some(member.id),
        })
        .await
        .unwrap();

    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "see attached".into(),
            metadata: serde_json::json!({ "artifact_sha256": artifact.sha256 }),
            content: None,
        })
        .await
        .unwrap();

    store
        .add_reference(NewReference {
            src_kind: RefSide::Message,
            src_id: msg.id.0,
            dst_kind: RefSide::Thread,
            dst_id: thread.id.0,
            relation: "relates_to".into(),
        })
        .await
        .unwrap();

    let res = client
        .get(format!("{base}/threads/{}/context", thread.id.0))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = res.json().await.unwrap();

    assert_eq!(body["workspace_id"], ws.id.0.to_string());
    assert_eq!(body["channel_id"], ch.id.0.to_string());
    assert_eq!(body["thread"]["state"], "in_review");
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["references"].as_array().unwrap().len(), 1);
    assert_eq!(body["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(body["artifacts"][0]["sha256"], artifact.sha256);
    assert_eq!(body["fsm"]["state"], "in_review");
    assert_eq!(body["fsm"]["transitions"].as_array().unwrap().len(), 1);
    assert_eq!(body["fsm"]["transitions"][0]["to_state"], "in_review");

    server.abort();
}

#[tokio::test]
async fn thread_context_edit_bodies_are_opt_in() {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(
        store.clone(),
        artifacts.clone(),
        bus,
        search,
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "edit-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
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
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("edited thread".into()),
        })
        .await
        .unwrap();

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("ctx".into()),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());

    let msg = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "first draft".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    store
        .edit_message(
            msg.id,
            member.id,
            EditMessage {
                body: "final wording".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
        )
        .await
        .unwrap();

    // Default: the edit is present but its heavy body copies are elided.
    let lean: serde_json::Value = client
        .get(format!("{base}/threads/{}/context", thread.id.0))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let edits = lean["message_edits"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert!(edits[0].get("editor_id").is_some());
    assert!(edits[0].get("edited_at").is_some());
    assert!(edits[0].get("body_before").is_none());
    assert!(edits[0].get("body_after").is_none());

    // Opt-in restores the full before/after bodies.
    let full: serde_json::Value = client
        .get(format!(
            "{base}/threads/{}/context?include_edits=true",
            thread.id.0
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let edits = full["message_edits"].as_array().unwrap();
    assert_eq!(edits[0]["body_before"], "first draft");
    assert_eq!(edits[0]["body_after"], "final wording");

    server.abort();
}

#[tokio::test]
async fn token_budget_folds_the_middle_and_records_elision() {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(
        store.clone(),
        artifacts.clone(),
        bus,
        search,
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "budget-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
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
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("budget thread".into()),
        })
        .await
        .unwrap();

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("ctx".into()),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());

    // Six fat messages (~500 body chars each) — each well over a tight per-message
    // budget. Distinct bodies so we can assert which survived.
    const N: usize = 6;
    for i in 0..N {
        let body = format!("msg{i}-{}", (b'a' + i as u8) as char).repeat(120);
        store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body,
                metadata: serde_json::json!({}),
                content: None,
            })
            .await
            .unwrap();
    }
    let opener_prefix = "msg0-a";
    let newest_prefix = format!("msg{}-{}", N - 1, (b'a' + (N - 1) as u8) as char);

    // A tight budget folds the middle: opener + recent tail kept, the rest elided.
    let folded: serde_json::Value = client
        .get(format!(
            "{base}/threads/{}/context?token_budget=400",
            thread.id.0
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let kept = folded["messages"].as_array().unwrap();
    assert!(kept.len() >= 2, "opener + newest always survive");
    assert!(kept.len() < N, "the middle was folded away");
    // Framing (first message) and recency (last message) are preserved.
    assert!(kept.first().unwrap()["body"]
        .as_str()
        .unwrap()
        .starts_with(opener_prefix));
    assert!(kept.last().unwrap()["body"]
        .as_str()
        .unwrap()
        .starts_with(&newest_prefix));
    let elision = &folded["elision"];
    assert!(!elision.is_null(), "an over-budget pack carries an elision");
    assert_eq!(
        elision["elided_message_count"].as_u64().unwrap() as usize,
        N - kept.len()
    );
    assert!(elision["elided_token_estimate"].as_u64().unwrap() > 0);
    assert!(elision["first_elided_id"].is_string());
    assert!(elision["summary"].as_str().unwrap().contains("elided"));

    // No budget → every message, no elision marker.
    let full: serde_json::Value = client
        .get(format!("{base}/threads/{}/context", thread.id.0))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(full["messages"].as_array().unwrap().len(), N);
    assert!(full.get("elision").is_none() || full["elision"].is_null());

    server.abort();
}
