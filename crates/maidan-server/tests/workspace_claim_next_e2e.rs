//! `POST /workspaces/{wid}/threads/claim-next` and its MCP twin
//! `claim_next_workspace_thread`: the next ready thread anywhere in the
//! caller's workspace, leased and fenced like the channel route's, never a
//! thread the caller may not read and never another tenant's. The channel
//! route on the shared `__dm__` channel is held to the same rule.

mod seeded_workspace;

use chrono::{DateTime, Utc};
use maidan_auth::capability;
use maidan_store::Store;
use maidan_types::{
    ChannelId, ChannelMemberRole, MemberId, MemberKind, NewChannel, NewThread, NewWorkspace,
    ThreadId, WorkspaceId,
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

async fn channel(store: &dyn Store, ws: WorkspaceId, name: &str, private: bool) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: name.into(),
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
            title: None,
        })
        .await
        .unwrap()
        .id
}

async fn agent_token(store: &dyn Store, ws: WorkspaceId, member_id: MemberId) -> String {
    token(
        store,
        ws,
        member_id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::THREAD_TRANSITION.into(),
        ],
    )
    .await
}

async fn claim_next(h: &Harness, bearer: &str, ws: WorkspaceId) -> (StatusCode, Value) {
    let res = h
        .send(
            Method::POST,
            &format!("/workspaces/{}/threads/claim-next", ws.0),
            bearer,
            Some(json!({})),
        )
        .await;
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

fn claimed_id(body: &Value) -> Option<String> {
    body["id"].as_str().map(str::to_string)
}

async fn call_tool(h: &Harness, bearer: &str, name: &str, arguments: Value) -> Value {
    h.send(
        Method::POST,
        "/mcp",
        bearer,
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
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

/// One call drains every channel the caller may read, oldest first, each claim
/// leased with the server default and fenced; private and DM threads the
/// caller may not read stay put.
#[tokio::test]
async fn one_call_takes_the_oldest_readable_thread_in_the_workspace() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let ws = workspace(store, "claims").await;
    let agent = member(store, ws, "agent", MemberKind::Agent).await;
    let insider = member(store, ws, "insider", MemberKind::Agent).await;
    let other = member(store, ws, "other", MemberKind::Human).await;
    let secret = channel(store, ws, "secret", true).await;
    store
        .add_channel_member(secret, insider, ChannelMemberRole::Member)
        .await
        .unwrap();
    let hidden = thread(store, secret).await;
    let dm = store
        .open_dm_conversation(ws, insider, other)
        .await
        .unwrap();
    let alpha = channel(store, ws, "alpha", false).await;
    let beta = channel(store, ws, "beta", false).await;
    let first = thread(store, beta).await;
    let second = thread(store, alpha).await;
    let bearer = agent_token(store, ws, agent).await;

    let (status, body) = claim_next(&h, &bearer, ws).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(claimed_id(&body), Some(first.0.to_string()));
    assert_eq!(body["assignee_id"], json!(agent.0.to_string()));
    assert!(
        body["claim_lease_id"].is_string(),
        "a fencing token: {body}"
    );
    assert!(body["pin"]["content_hash"].is_string(), "a pin: {body}");
    let expires: DateTime<Utc> = body["assignment_expires_at"]
        .as_str()
        .expect("leased")
        .parse()
        .unwrap();
    let lease = expires - Utc::now();
    assert!(
        lease > chrono::Duration::seconds(500) && lease <= chrono::Duration::seconds(600),
        "the default lease, got {lease}"
    );

    let (_, body) = claim_next(&h, &bearer, ws).await;
    assert_eq!(claimed_id(&body), Some(second.0.to_string()));
    let (status, body) = claim_next(&h, &bearer, ws).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        Value::Null,
        "the private thread and the DM are not the agent's to take"
    );
    for untouched in [hidden, dm.thread_id] {
        assert_eq!(store.get_thread(untouched).await.unwrap().assignee_id, None);
    }

    let insider_bearer = agent_token(store, ws, insider).await;
    let res = call_tool(
        &h,
        &insider_bearer,
        "claim_next_workspace_thread",
        json!({ "workspace_id": ws.0 }),
    )
    .await;
    assert_eq!(
        claimed_id(&tool_json(&res)),
        Some(hidden.0.to_string()),
        "the MCP twin gives the insider the private thread first"
    );
    let res = call_tool(
        &h,
        &insider_bearer,
        "claim_next_workspace_thread",
        json!({ "workspace_id": ws.0 }),
    )
    .await;
    assert_eq!(
        claimed_id(&tool_json(&res)),
        Some(dm.thread_id.0.to_string()),
        "then its own DM"
    );
}

/// Neither surface lets one tenant's token claim in another's workspace, and
/// each tenant's claims stay in its own.
#[tokio::test]
async fn two_tenants_never_claim_each_others_work() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let ws_a = workspace(store, "tenant-a").await;
    let ws_b = workspace(store, "tenant-b").await;
    let agent_a = member(store, ws_a, "agent", MemberKind::Agent).await;
    let agent_b = member(store, ws_b, "agent", MemberKind::Agent).await;
    let b_thread = thread(store, channel(store, ws_b, "work", false).await).await;
    let a_thread = thread(store, channel(store, ws_a, "work", false).await).await;
    let bearer_a = agent_token(store, ws_a, agent_a).await;
    let bearer_b = agent_token(store, ws_b, agent_b).await;

    let (status, body) = claim_next(&h, &bearer_a, ws_b).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let res = call_tool(
        &h,
        &bearer_a,
        "claim_next_workspace_thread",
        json!({ "workspace_id": ws_b.0 }),
    )
    .await;
    assert!(
        res.get("error").is_some() || res["result"]["isError"] == true,
        "the MCP twin refuses another workspace: {res}"
    );
    assert!(!res.to_string().contains(&b_thread.0.to_string()));
    assert_eq!(store.get_thread(b_thread).await.unwrap().assignee_id, None);

    let (_, body) = claim_next(&h, &bearer_a, ws_a).await;
    assert_eq!(claimed_id(&body), Some(a_thread.0.to_string()));
    let (_, body) = claim_next(&h, &bearer_a, ws_a).await;
    assert_eq!(body, Value::Null, "A drained its own work and nothing else");
    let (_, body) = claim_next(&h, &bearer_b, ws_b).await;
    assert_eq!(claimed_id(&body), Some(b_thread.0.to_string()));
}

/// The channel route passed `__dm__` as a channel every member may use, then
/// handed out its oldest thread, whoever's DM it was.
#[tokio::test]
async fn the_dm_channel_does_not_hand_out_someone_elses_dm() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let ws = workspace(store, "dm").await;
    let alice = member(store, ws, "alice", MemberKind::Human).await;
    let bob = member(store, ws, "bob", MemberKind::Agent).await;
    let carol = member(store, ws, "carol", MemberKind::Agent).await;
    let dm = store.open_dm_conversation(ws, alice, bob).await.unwrap();
    let dm_channel = store.get_thread(dm.thread_id).await.unwrap().channel_id;
    let bearer = agent_token(store, ws, carol).await;

    let res = h
        .send(
            Method::POST,
            &format!("/channels/{}/threads/claim-next", dm_channel.0),
            &bearer,
            Some(json!({})),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value = res.json().await.unwrap();
    assert_eq!(
        body,
        Value::Null,
        "carol was handed alice and bob's DM: {body}"
    );
    let res = call_tool(
        &h,
        &bearer,
        "claim_next_thread",
        json!({ "channel_id": dm_channel.0 }),
    )
    .await;
    assert_eq!(tool_json(&res), Value::Null, "nor over MCP");
    assert_eq!(
        store.get_thread(dm.thread_id).await.unwrap().assignee_id,
        None
    );

    let bob_bearer = agent_token(store, ws, bob).await;
    let (_, body) = claim_next(&h, &bob_bearer, ws).await;
    assert_eq!(
        claimed_id(&body),
        Some(dm.thread_id.0.to_string()),
        "a participant may take it"
    );
}
