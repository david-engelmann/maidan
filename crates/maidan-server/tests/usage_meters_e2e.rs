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
