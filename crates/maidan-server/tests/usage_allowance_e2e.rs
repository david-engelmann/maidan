//! The free allowance (`MAIDAN_FREE_ALLOWANCE`): a workspace past it on a
//! meter is marked once a month and its admins are told; the workspace's
//! usage and the operator's month show it; another workspace is untouched,
//! and nothing is cut off.

mod seeded_workspace;

use maidan_auth::capability;
use maidan_server::AppState;
use maidan_store::Store;
use maidan_types::{EventKind, MemberKind, NewWorkspace, UsageAllowance, UsageMeter, WorkspaceId};
use reqwest::{Method, StatusCode};
use seeded_workspace::{member, spawn_with, token, Harness};
use serde_json::Value;

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

async fn get(h: &Harness, bearer: &str, path: &str) -> (StatusCode, Value) {
    let res = h.send(Method::GET, path, bearer, None).await;
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

fn meter<'a>(usage: &'a Value, name: &str) -> &'a Value {
    usage["meters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["meter"] == name)
        .unwrap()
}

#[tokio::test]
async fn a_workspace_over_its_allowance_is_marked_once_and_its_admins_told() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = std::sync::Arc::new(UsageAllowance::parse("requests=2").unwrap());
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let ops = workspace(store, "ops").await;
    let a = workspace(store, "a").await;
    let b = workspace(store, "b").await;
    let operator = member(store, ops, "operator", MemberKind::Human).await;
    let operator = token(
        store,
        ops,
        operator,
        vec![capability::OPERATOR_GLOBAL.into()],
    )
    .await;
    let admin_caps = || {
        vec![
            capability::WORKSPACE_READ.to_string(),
            capability::TOKEN_ADMIN.to_string(),
        ]
    };
    let a_admin = member(store, a, "admin", MemberKind::Human).await;
    let a_token = token(store, a, a_admin, admin_caps()).await;
    let b_admin = member(store, b, "admin", MemberKind::Human).await;
    let b_token = token(store, b, b_admin, admin_caps()).await;
    let usage = |ws: WorkspaceId| format!("/workspaces/{}/usage-meters", ws.0);

    // A goes past its two requests; B stays inside.
    for _ in 0..3 {
        let (status, _) = get(&h, &a_token, &format!("/workspaces/{}", a.0)).await;
        assert_eq!(status, StatusCode::OK, "nothing is cut off");
    }
    let (status, _) = get(&h, &b_token, &format!("/workspaces/{}", b.0)).await;
    assert_eq!(status, StatusCode::OK);

    let (_, _, marked) = maidan_server::usage_meters::tick(&state).await;
    assert_eq!(marked, 1, "A, on requests");
    let (_, _, marked) = maidan_server::usage_meters::tick(&state).await;
    assert_eq!(marked, 0, "once a month");

    // A's admin is told; B's is not.
    let told = store.list_notifications(a_admin, false, 10).await.unwrap();
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].kind, EventKind::UsageOverAllowance);
    assert!(store
        .list_notifications(b_admin, false, 10)
        .await
        .unwrap()
        .is_empty());

    // A's month shows it over; B reads its own, not A's.
    let (status, a_usage) = get(&h, &a_token, &usage(a)).await;
    assert_eq!(status, StatusCode::OK, "{a_usage}");
    let requests = meter(&a_usage, "requests");
    assert_eq!(requests["allowance"], 2);
    assert_eq!(requests["over"], true, "{a_usage}");
    assert!(requests["quantity"].as_i64().unwrap() >= 3);
    assert_eq!(meter(&a_usage, "mcp_calls")["allowance"], Value::Null);
    let (status, _) = get(&h, &b_token, &usage(a)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, b_usage) = get(&h, &b_token, &usage(b)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(meter(&b_usage, "requests")["over"], false, "{b_usage}");

    // The operator's month has both; a tenant may not read it; a bad month
    // is refused.
    let (status, month) = get(&h, &operator, "/operator/usage-meters").await;
    assert_eq!(status, StatusCode::OK, "{month}");
    let over: Vec<_> = month
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| meter(w, UsageMeter::Requests.as_str())["over"] == true)
        .map(|w| w["workspace_id"].clone())
        .collect();
    assert_eq!(over, vec![serde_json::json!(a.0)]);
    let (status, _) = get(&h, &a_token, "/operator/usage-meters").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = get(&h, &operator, "/operator/usage-meters?month=October").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
