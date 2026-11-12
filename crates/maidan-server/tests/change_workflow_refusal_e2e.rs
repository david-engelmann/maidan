//! `MAIDAN_CHANGE_REFUSE_WORKFLOW_PATHS`: an operator whose GitHub delivery
//! token has no Workflows permission turns it on, and `PUT
//! /threads/{id}/result` then refuses, with a 422 naming the files, a change
//! whose branch delivery would edit `.github/workflows/`. Without it the
//! builder heard nothing until the push failed in the egress worker, and the
//! task sat in review. Off, the same result is stored; a change that touches
//! no workflow file is stored either way.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, CHANGE_RESULT_KIND,
    WAITER_RESULT_SCHEMA,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

const WORKFLOW_DIFF: &str = "diff --git a/.github/workflows/test.yml b/.github/workflows/test.yml\n--- a/.github/workflows/test.yml\n+++ b/.github/workflows/test.yml\n@@ -1,2 +1,2 @@\n name: test\n-on: push\n+on: [push, pull_request]\n";
const SOURCE_DIFF: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n";

fn change(diff: &str) -> Value {
    json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": CHANGE_RESULT_KIND,
        "status": "changed",
        "base_sha": "0123456789abcdef0123456789abcdef01234567",
        "branch": "feature/agent-fix-ci-1a2b",
        "diff": diff,
        "deliver_to": [
            {"surface": "github_branch", "repo": "acme/app", "branch": "feature/agent-fix-ci-1a2b", "base": "dev"},
        ],
    })
}

#[tokio::test]
async fn a_change_editing_a_workflow_file_is_refused_with_422_only_while_the_switch_is_on() {
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
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let builder = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "builder".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let lease = store.claim_lease_for(thread.id, builder.id).await.unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: builder.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(16)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let mcp = state.mcp.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    let client = reqwest::Client::new();
    let put = |result: Value| {
        client
            .put(format!("http://{addr}/threads/{}/result", thread.id.0))
            .bearer_auth(secret.as_str())
            .json(&json!({ "claim_lease_id": lease.0, "result": result }))
            .send()
    };
    let workflow_and_source = format!("{SOURCE_DIFF}{WORKFLOW_DIFF}");

    mcp.set_refuse_workflow_paths(true);
    let refused = put(change(&workflow_and_source)).await.unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let text = refused.text().await.unwrap();
    for part in [
        "`.github/workflows/test.yml`",
        "MAIDAN_CHANGE_REFUSE_WORKFLOW_PATHS",
        "Drop the workflow edit",
        "Workflows permission",
    ] {
        assert!(text.contains(part), "the refusal says {part}: {text}");
    }
    assert!(
        !text.contains("src/lib.rs"),
        "only the refused path is named: {text}"
    );
    assert!(
        store.get_thread_result(thread.id).await.unwrap().is_none(),
        "a refused result is not stored"
    );
    let source_only = put(change(SOURCE_DIFF)).await.unwrap();
    assert_eq!(
        source_only.status(),
        StatusCode::OK,
        "a change that edits no workflow file is stored while the switch is on"
    );

    mcp.set_refuse_workflow_paths(false);
    let accepted = put(change(&workflow_and_source)).await.unwrap();
    assert_eq!(
        accepted.status(),
        StatusCode::OK,
        "off, the same change is stored as before"
    );
    let stored = store.get_thread_result(thread.id).await.unwrap().unwrap();
    assert_eq!(stored.result["diff"], json!(workflow_and_source));
    let source_only = put(change(SOURCE_DIFF)).await.unwrap();
    assert_eq!(source_only.status(), StatusCode::OK);
}
