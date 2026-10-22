//! `GET /workspaces/{id}/thread-lookup` and the MCP `find_thread`: a pull
//! request or Slack channel finds its thread for a caller who may read it,
//! and answers not found for a private channel's thread or another tenant's.

mod seeded_workspace;

use maidan_auth::capability;
use maidan_store::Store;
use maidan_types::{
    ChannelId, MemberKind, NewChannel, NewSlackChannelLink, NewThread, NewWorkspace, ThreadId,
    WorkspaceId,
};
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

async fn channel(store: &dyn Store, ws: WorkspaceId, private: bool) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: if private { "private" } else { "work" }.into(),
            topic: None,
            private,
        })
        .await
        .unwrap()
        .id
}

async fn thread(store: &dyn Store, channel_id: ChannelId) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id
}

async fn lookup(h: &Harness, bearer: &str, ws: WorkspaceId, query: &str) -> (StatusCode, Value) {
    let res = h
        .send(
            Method::GET,
            &format!("/workspaces/{}/thread-lookup?{query}", ws.0),
            bearer,
            None,
        )
        .await;
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

async fn find_thread(h: &Harness, bearer: &str, arguments: Value) -> Value {
    h.send(
        Method::POST,
        "/mcp",
        bearer,
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "find_thread", "arguments": arguments }
        })),
    )
    .await
    .json()
    .await
    .unwrap()
}

fn tool_json(res: &Value) -> Value {
    let text = res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no tool result: {res}"));
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn a_reference_finds_a_thread_the_caller_may_read_and_nothing_else() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let a = workspace(store, "a").await;
    let b = workspace(store, "b").await;
    let reader = member(store, a, "reader", MemberKind::Agent).await;
    let outsider = member(store, b, "outsider", MemberKind::Agent).await;
    let public = channel(store, a, false).await;
    let private = channel(store, a, true).await;
    let task = thread(store, public).await;
    let hidden = thread(store, private).await;
    store
        .register_change_pull(a, task, "example/app", 7, None)
        .await
        .unwrap();
    store
        .register_change_pull(a, hidden, "example/app", 8, None)
        .await
        .unwrap();
    store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: "C0AAA".into(),
            workspace_id: a,
            channel_id: public,
            thread_id: task,
            member_id: reader,
        })
        .await
        .unwrap();
    let read = vec![capability::WORKSPACE_READ.into()];
    let bearer = token(store, a, reader, read.clone()).await;
    let other = token(store, b, outsider, read).await;

    for query in ["repo=Example/App&pr_number=7", "slack_channel=C0AAA"] {
        let (status, body) = lookup(&h, &bearer, a, query).await;
        assert_eq!(status, StatusCode::OK, "{query}: {body}");
        assert_eq!(body["id"], json!(task.0), "{query}");
    }
    let found = tool_json(
        &find_thread(
            &h,
            &bearer,
            json!({ "repo": "example/app", "pr_number": 7 }),
        )
        .await,
    );
    assert_eq!(found["id"], json!(task.0), "the twin, workspace defaulted");

    // A private channel's thread answers as no thread does.
    let (status, _) = lookup(&h, &bearer, a, "repo=example/app&pr_number=8").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = lookup(&h, &bearer, a, "repo=example/app&pr_number=9").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let hidden_found = tool_json(
        &find_thread(
            &h,
            &bearer,
            json!({ "repo": "example/app", "pr_number": 8 }),
        )
        .await,
    );
    assert!(hidden_found.is_null(), "{hidden_found}");

    // Another tenant finds nothing in its own workspace and may not ask in A's.
    let (status, _) = lookup(&h, &other, b, "repo=example/app&pr_number=7").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = lookup(&h, &other, a, "repo=example/app&pr_number=7").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let none = tool_json(&find_thread(&h, &other, json!({ "slack_channel": "C0AAA" })).await);
    assert!(none.is_null(), "{none}");
    let refused = find_thread(
        &h,
        &other,
        json!({ "workspace_id": a.0, "slack_channel": "C0AAA" }),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32003, "{refused}");

    // A malformed lookup is refused before anything is read.
    let (status, _) = lookup(&h, &bearer, a, "slack_ts=1700000000.000100").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let refused = find_thread(&h, &bearer, json!({ "repo": "example/app" })).await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
}
