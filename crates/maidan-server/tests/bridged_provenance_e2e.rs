//! A message's provenance is the server's to set (RED-007). Over REST a member
//! can neither forge one on a post nor change or strip one on an edit, and the
//! message reads show it read-only. Runs with auth bypassed
//! (`AppState::for_tests`), acting as a member by header.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, MessageId, MessageOrigin, MessageProvenance, NewChannel, NewMember, NewMessage,
    NewThread, NewWorkspace,
};
use reqwest::StatusCode;
use serde_json::json;

#[tokio::test]
async fn a_member_cannot_forge_or_strip_provenance_over_rest() {
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let ws = store
        .create_workspace(NewWorkspace {
            name: "acme".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "linker".into(),
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
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let as_member = member.id.0.to_string();

    // Bridge keys in metadata make a member's post no bridged message.
    let resp = client
        .post(format!("{base}/threads/{}/messages", thread.id.0))
        .header("maidan-test-member-id", &as_member)
        .json(&json!({
            "body": "U9: from slack",
            "metadata": { "slack": { "user": "U9", "channel": "C1" }, "github": { "user": "octocat" } },
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let posted: serde_json::Value = resp.json().await.unwrap();
    assert!(posted.get("provenance").is_none(), "{posted}");
    let id = MessageId(posted["id"].as_str().unwrap().parse().unwrap());
    assert!(store.get_message(id).await.unwrap().provenance.is_none());

    // Provenance itself is no field a post takes.
    let resp = client
        .post(format!("{base}/threads/{}/messages", thread.id.0))
        .header("maidan-test-member-id", &as_member)
        .json(&json!({
            "body": "forged",
            "provenance": { "origin": "github", "external_id": "1" },
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_client_error(), "{}", resp.status());

    // A message Slack ingress posted as this member keeps its provenance
    // through the member's edits, including one that clears the metadata.
    let event = MessageProvenance::delivered(MessageOrigin::Slack, "Ev42");
    let (bridged, _) = store
        .post_bridged_message_with_event(
            NewMessage {
                thread_id: thread.id,
                author_id: member.id,
                body: "U9: hello".into(),
                metadata: json!({ "slack": { "user": "U9" } }),
                content: None,
            },
            event.clone(),
        )
        .await
        .unwrap()
        .unwrap();
    let resp = client
        .patch(format!("{base}/messages/{}", bridged.id.0))
        .header("maidan-test-member-id", &as_member)
        .json(&json!({ "body": "U9: hello!", "metadata": {} }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let edited: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        edited["provenance"],
        json!({ "origin": "slack", "external_id": "Ev42" })
    );
    let resp = client
        .patch(format!("{base}/messages/{}", bridged.id.0))
        .header("maidan-test-member-id", &as_member)
        .json(&json!({ "body": "mine now", "provenance": null }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_client_error(), "{}", resp.status());
    let after = store.get_message(bridged.id).await.unwrap();
    assert_eq!(after.body, "U9: hello!");
    assert_eq!(after.provenance.as_ref(), Some(&event));

    // Reads show it.
    let listed: serde_json::Value = client
        .get(format!("{base}/threads/{}/messages", thread.id.0))
        .header("maidan-test-member-id", &as_member)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let rows = listed
        .as_array()
        .or_else(|| listed["messages"].as_array())
        .or_else(|| listed["items"].as_array())
        .unwrap_or_else(|| panic!("a list of messages: {listed}"));
    let shown = rows
        .iter()
        .find(|m| m["id"] == json!(bridged.id.0))
        .unwrap();
    assert_eq!(
        shown["provenance"],
        json!({ "origin": "slack", "external_id": "Ev42" })
    );
    server.abort();
}
