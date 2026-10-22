//! Usage meters: each authenticated request and each MCP tool call counts for
//! the caller's workspace, in memory, until a flush writes the day's totals.
//! Another workspace's calls never count for it, and probes are not metered.

mod seeded_workspace;

use maidan_auth::capability;
use maidan_server::AppState;
use maidan_store::Store;
use maidan_types::{MemberKind, NewWorkspace, UsageMeter, WorkspaceId};
use reqwest::{Method, StatusCode};
use seeded_workspace::{member, spawn_with, token, Harness};
use serde_json::json;

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

async fn whoami(h: &Harness, bearer: &str) {
    let res = h
        .send(
            Method::POST,
            "/mcp",
            bearer,
            Some(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "whoami", "arguments": {} }
            })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
}

async fn meters(store: &dyn Store, ws: WorkspaceId) -> Vec<(UsageMeter, i64)> {
    let today = chrono::Utc::now().date_naive();
    store
        .list_usage_meters(ws, today)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.meter, row.quantity))
        .collect()
}

#[tokio::test]
async fn requests_and_mcp_calls_count_for_the_callers_workspace_only() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| state = Some(s.clone())).await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let a = workspace(store, "a").await;
    let b = workspace(store, "b").await;
    let caps = || vec![capability::WORKSPACE_READ.to_string()];
    let a_member = member(store, a, "agent", MemberKind::Agent).await;
    let a_token = token(store, a, a_member, caps()).await;
    let b_member = member(store, b, "agent", MemberKind::Agent).await;
    let b_token = token(store, b, b_member, caps()).await;

    for _ in 0..3 {
        let res = h
            .send(Method::GET, &format!("/workspaces/{}", a.0), &a_token, None)
            .await;
        assert_eq!(res.status(), StatusCode::OK);
    }
    whoami(&h, &a_token).await;
    whoami(&h, &a_token).await;
    let res = h
        .send(Method::GET, &format!("/workspaces/{}", b.0), &b_token, None)
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    // A probe is not metered.
    let res = h.client.get(h.url("/health")).send().await.unwrap();
    assert!(res.status().is_success());

    assert!(
        meters(store, a).await.is_empty(),
        "nothing is written before a flush"
    );
    assert!(maidan_server::usage_meters::flush_once(&state).await > 0);
    // Each MCP call is also an authenticated request.
    assert_eq!(
        meters(store, a).await,
        vec![(UsageMeter::McpCalls, 2), (UsageMeter::Requests, 5)]
    );
    assert_eq!(meters(store, b).await, vec![(UsageMeter::Requests, 1)]);

    // A second flush adds to the day's totals.
    whoami(&h, &b_token).await;
    maidan_server::usage_meters::flush_once(&state).await;
    assert_eq!(
        meters(store, b).await,
        vec![(UsageMeter::McpCalls, 1), (UsageMeter::Requests, 2)]
    );
    assert_eq!(meters(store, a).await.len(), 2, "A unchanged");
    assert_eq!(maidan_server::usage_meters::flush_once(&state).await, 0);
}

/// A semantic search embeds its query, and that embedding is metered for the
/// searcher's workspace.
#[tokio::test]
async fn a_semantic_search_counts_one_embedding_for_the_searchers_workspace() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| state = Some(s.clone())).await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let a = workspace(store, "a").await;
    let b = workspace(store, "b").await;
    let reader = member(store, a, "agent", MemberKind::Agent).await;
    let bearer = token(
        store,
        a,
        reader,
        vec![
            capability::WORKSPACE_READ.to_string(),
            capability::SEARCH_QUERY.to_string(),
        ],
    )
    .await;
    let res = h
        .client
        .get(h.url(&format!("/workspaces/{}/search", a.0)))
        .bearer_auth(&bearer)
        .query(&[("q", "anything"), ("mode", "semantic")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    maidan_server::usage_meters::flush_once(&state).await;
    assert!(meters(store, a)
        .await
        .contains(&(UsageMeter::EmbeddingCalls, 1)));
    assert!(meters(store, b).await.is_empty());
}

/// What each workspace stores is measured once a day, its own messages and
/// artifacts only, and a second tick the same day measures nothing new.
#[tokio::test]
async fn stored_bytes_are_measured_once_a_day_per_workspace() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| state = Some(s.clone())).await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let a = workspace(store, "a").await;
    let b = workspace(store, "b").await;
    let author = member(store, a, "agent", MemberKind::Agent).await;
    let channel = store
        .create_channel(maidan_types::NewChannel {
            workspace_id: a,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(maidan_types::NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .post_message(maidan_types::NewMessage {
            thread_id: thread.id,
            author_id: author,
            body: "twelve bytes".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();

    assert!(maidan_server::usage_meters::measure_stored_bytes(&state).await >= 2);
    let stored = |ws| async move {
        meters(store, ws)
            .await
            .into_iter()
            .find(|(meter, _)| *meter == UsageMeter::StoredBytes)
            .map(|(_, quantity)| quantity)
    };
    assert_eq!(stored(a).await, Some(12));
    assert_eq!(
        stored(b).await,
        Some(0),
        "B is measured, and stores nothing of A's"
    );
    assert_eq!(
        maidan_server::usage_meters::measure_stored_bytes(&state).await,
        0,
        "measured once a day"
    );
}
