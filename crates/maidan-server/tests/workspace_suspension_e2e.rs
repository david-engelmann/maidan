//! `/operator/workspaces/{id}/suspension`: an operator suspends a workspace and
//! lifts it. While suspended, its tokens are refused with a 403 on REST and
//! MCP, another workspace is untouched, and both acts are audited.

mod seeded_workspace;

use maidan_auth::capability;
use maidan_store::Store;
use maidan_types::{MemberKind, NewWorkspace, WorkspaceId};
use reqwest::{Method, StatusCode};
use seeded_workspace::{member, spawn, token, Harness};
use serde_json::{json, Value};

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

async fn send(
    h: &Harness,
    method: Method,
    path: &str,
    bearer: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let res = h.send(method, path, bearer, body).await;
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

async fn whoami(h: &Harness, bearer: &str) -> (StatusCode, Value) {
    send(
        h,
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
    .await
}

#[tokio::test]
async fn suspending_a_workspace_refuses_its_tokens_and_leaves_another_working() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let ops = workspace(store, "ops").await;
    let a = workspace(store, "a").await;
    let b = workspace(store, "b").await;
    let operator = member(store, ops, "operator", MemberKind::Human).await;
    let operator = token(
        store,
        ops,
        operator,
        vec![
            capability::OPERATOR_GLOBAL.into(),
            capability::WORKSPACE_READ.into(),
        ],
    )
    .await;
    let caps = || {
        vec![
            capability::WORKSPACE_READ.to_string(),
            capability::TOKEN_ADMIN.to_string(),
        ]
    };
    let a_member = member(store, a, "agent", MemberKind::Agent).await;
    let a_token = token(store, a, a_member, caps()).await;
    let b_member = member(store, b, "agent", MemberKind::Agent).await;
    let b_token = token(store, b, b_member, caps()).await;
    let path = |ws: WorkspaceId| format!("/operator/workspaces/{}/suspension", ws.0);
    let reads = |ws: WorkspaceId| format!("/workspaces/{}", ws.0);

    // Only an operator suspends, and never its own workspace.
    let body = json!({ "reason": "abuse report 17" });
    let (status, _) = send(&h, Method::PUT, &path(a), &b_token, Some(body.clone())).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a tenant admin may not suspend"
    );
    let (status, _) = send(&h, Method::PUT, &path(ops), &operator, Some(body.clone())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "not its own workspace");
    let (status, _) = send(&h, Method::GET, &path(a), &operator, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "not suspended yet");

    let (status, suspended) = send(&h, Method::PUT, &path(a), &operator, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{suspended}");
    assert_eq!(suspended["reason"], "abuse report 17");
    let (status, got) = send(&h, Method::GET, &path(a), &operator, None).await;
    assert_eq!(
        (status, &got["workspace_id"]),
        (StatusCode::OK, &json!(a.0))
    );

    // A's token is refused on REST and MCP with the reason; B works.
    let (status, refused) = send(&h, Method::GET, &reads(a), &a_token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(
        refused.to_string().contains("workspace suspended"),
        "{refused}"
    );
    let (status, refused) = whoami(&h, &a_token).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    let (status, _) = send(&h, Method::GET, &reads(b), &b_token, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, answer) = whoami(&h, &b_token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(answer["result"].is_object(), "{answer}");

    // Lifting restores A; lifting again lifts nothing.
    let (status, lifted) = send(&h, Method::DELETE, &path(a), &operator, None).await;
    assert_eq!((status, &lifted["lifted"]), (StatusCode::OK, &json!(true)));
    let (status, _) = send(&h, Method::GET, &reads(a), &a_token, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, lifted) = send(&h, Method::DELETE, &path(a), &operator, None).await;
    assert_eq!((status, &lifted["lifted"]), (StatusCode::OK, &json!(false)));

    // Both acts are in A's audit log, attributed to the operator.
    let actions: Vec<String> = store
        .list_audit_for_workspace(a, 50)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.action)
        .collect();
    for action in ["workspace.suspend", "workspace.suspension_lift"] {
        assert_eq!(
            actions.iter().filter(|a| *a == action).count(),
            1,
            "{action} in {actions:?}"
        );
    }
}
