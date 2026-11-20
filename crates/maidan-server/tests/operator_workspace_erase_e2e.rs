//! `DELETE /operator/workspaces/{id}`: an operator erases a workspace whose
//! own admin cannot. Only an operator may, never its own workspace, never
//! under a legal hold, and another workspace is untouched.

mod seeded_workspace;

use maidan_auth::capability;
use maidan_store::Store;
use maidan_types::{MemberKind, NewChannel, NewWorkspace, WorkspaceId};
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

fn admin_caps() -> Vec<String> {
    vec![
        capability::WORKSPACE_READ.to_string(),
        capability::TOKEN_ADMIN.to_string(),
    ]
}

#[tokio::test]
async fn an_operator_erases_another_workspace_and_only_an_operator_may() {
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
    let a_member = member(store, a, "person", MemberKind::Human).await;
    let a_token = token(store, a, a_member, admin_caps()).await;
    let b_member = member(store, b, "person", MemberKind::Human).await;
    let b_token = token(store, b, b_member, admin_caps()).await;
    store
        .create_channel(NewChannel {
            workspace_id: a,
            name: "launch".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    let path = |ws: WorkspaceId| format!("/operator/workspaces/{}", ws.0);
    let reads = |ws: WorkspaceId| format!("/workspaces/{}", ws.0);
    let body =
        |ws: WorkspaceId| json!({ "confirm_workspace_id": ws.0, "reason": "orphaned test tenant" });

    // A tenant's admin is not an operator, in its own workspace or another's.
    for target in [a, b] {
        let (status, _) = send(
            &h,
            Method::DELETE,
            &path(target),
            &b_token,
            Some(body(target)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a tenant admin may not use the operator route"
        );
    }

    // The operator's mistakes erase nothing.
    let (status, _) = send(&h, Method::DELETE, &path(ops), &operator, Some(body(ops))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "not its own workspace");
    let (status, _) = send(&h, Method::DELETE, &path(a), &operator, Some(body(b))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "confirm names another id");
    let blank = json!({ "confirm_workspace_id": a.0, "reason": "  " });
    let (status, _) = send(&h, Method::DELETE, &path(a), &operator, Some(blank)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a reason is required");
    let nowhere = WorkspaceId(uuid::Uuid::now_v7());
    let (status, _) = send(
        &h,
        Method::DELETE,
        &path(nowhere),
        &operator,
        Some(body(nowhere)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an unknown workspace");
    let (status, _) = send(&h, Method::GET, &reads(a), &a_token, None).await;
    assert_eq!(status, StatusCode::OK, "A is still there");

    // A legal hold refuses the operator as it refuses the workspace's admin.
    let (status, hold) = send(
        &h,
        Method::POST,
        &format!("/workspaces/{}/legal-holds", a.0),
        &a_token,
        Some(json!({ "reason": "matter 12" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hold}");
    let (status, refused) = send(&h, Method::DELETE, &path(a), &operator, Some(body(a))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    let (status, _) = send(
        &h,
        Method::DELETE,
        &format!(
            "/workspaces/{}/legal-holds/{}",
            a.0,
            hold["id"].as_str().unwrap()
        ),
        &a_token,
        None,
    )
    .await;
    assert!(status.is_success(), "the hold lifts: {status}");

    // The erase removes A and its credentials, and leaves B alone.
    let (status, erased) = send(&h, Method::DELETE, &path(a), &operator, Some(body(a))).await;
    assert_eq!(status, StatusCode::OK, "{erased}");
    assert_eq!(erased["workspace_erased"], true, "{erased}");
    assert!(matches!(
        store.get_workspace(a).await,
        Err(maidan_store::StoreError::NotFound)
    ));
    let (status, _) = send(&h, Method::GET, &reads(a), &a_token, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "A's token went with it");
    let (status, _) = send(&h, Method::GET, &reads(b), &b_token, None).await;
    assert_eq!(status, StatusCode::OK, "B is untouched");
    let (status, _) = send(&h, Method::DELETE, &path(a), &operator, Some(body(a))).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a second erase finds nothing"
    );

    // The audit row names the operator's reason.
    let audit = store.list_audit(50).await.unwrap();
    let row = audit
        .iter()
        .find(|e| e.action == "workspace.erase")
        .expect("the erase is audited");
    assert_eq!(row.metadata["by"], "operator", "{:?}", row.metadata);
    assert_eq!(row.metadata["reason"], "orphaned test tenant");
    assert_eq!(row.target_id, Some(a.0));
}
