//! `GET /workspaces/{wid}/queue-depth`, `GET /workspaces/{wid}/occupancy` and
//! their MCP twins: the channel counts summed over every channel the caller may
//! read, by the rule `claim_next_workspace_thread` applies, never another
//! tenant's. `wait_for_ready` without a channel wakes by the same read rule.

mod seeded_workspace;

use maidan_auth::capability;
use maidan_store::Store;
use maidan_types::{
    ChannelId, ChannelMemberRole, MemberId, MemberKind, NewChannel, NewThread, NewWorkspace,
    ThreadId, WorkspaceId,
};
use reqwest::{Method, StatusCode};
use seeded_workspace::{member, spawn, spawn_with, token, Harness};
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

async fn get(h: &Harness, bearer: &str, path: &str) -> (StatusCode, Value) {
    let res = h.send(Method::GET, path, bearer, None).await;
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
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

/// Both surfaces, for one reader: (depth, occupancy) over REST then over MCP,
/// asserted equal, so each test states a count once.
async fn counts(h: &Harness, bearer: &str, ws: WorkspaceId) -> (Value, Value) {
    let (status, depth) = get(h, bearer, &format!("/workspaces/{}/queue-depth", ws.0)).await;
    assert_eq!(status, StatusCode::OK, "{depth}");
    let (status, occupancy) = get(h, bearer, &format!("/workspaces/{}/occupancy", ws.0)).await;
    assert_eq!(status, StatusCode::OK, "{occupancy}");
    let args = json!({ "workspace_id": ws.0 });
    let mcp_depth =
        tool_json(&call_tool(h, bearer, "get_workspace_queue_depth", args.clone()).await);
    let mcp_occupancy = tool_json(&call_tool(h, bearer, "get_workspace_occupancy", args).await);
    assert_eq!(depth, mcp_depth, "the MCP depth matches REST");
    assert_eq!(occupancy, mcp_occupancy, "the MCP occupancy matches REST");
    (depth, occupancy)
}

/// A private channel's threads count for its members and not for anyone else,
/// and a DM's only for its two participants, on the workspace route and on the
/// channel route over the shared `__dm__` channel.
#[tokio::test]
async fn the_workspace_counts_only_what_the_caller_may_read() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let ws = workspace(store, "counts").await;
    let outsider = member(store, ws, "outsider", MemberKind::Agent).await;
    let insider = member(store, ws, "insider", MemberKind::Agent).await;
    let other = member(store, ws, "other", MemberKind::Human).await;
    let open = channel(store, ws, "open", false).await;
    let secret = channel(store, ws, "secret", true).await;
    store
        .add_channel_member(secret, insider, ChannelMemberRole::Member)
        .await
        .unwrap();
    thread(store, open).await;
    thread(store, open).await;
    thread(store, secret).await;
    let dm = store
        .open_dm_conversation(ws, insider, other)
        .await
        .unwrap();
    let dm_channel = store.get_thread(dm.thread_id).await.unwrap().channel_id;

    let outsider_bearer = agent_token(store, ws, outsider).await;
    let (depth, occupancy) = counts(&h, &outsider_bearer, ws).await;
    assert_eq!(
        depth["open"], 2,
        "the outsider sees the open channel only: {depth}"
    );
    assert_eq!(depth["ready"], 2);
    assert_eq!(occupancy["open"], 2);
    assert_eq!(occupancy["queued"], 2);

    let insider_bearer = agent_token(store, ws, insider).await;
    let (depth, occupancy) = counts(&h, &insider_bearer, ws).await;
    assert_eq!(
        depth["open"], 4,
        "the open channel, the private one and its own DM: {depth}"
    );
    assert_eq!(depth["ready"], 4);
    assert_eq!(occupancy["queued"], 4);

    let (status, channel_depth) = get(
        &h,
        &outsider_bearer,
        &format!("/channels/{}/queue-depth", dm_channel.0),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{channel_depth}");
    assert_eq!(
        channel_depth["open"], 0,
        "the outsider's count of the DM channel holds nobody's DM: {channel_depth}"
    );
    let (_, channel_depth) = get(
        &h,
        &insider_bearer,
        &format!("/channels/{}/queue-depth", dm_channel.0),
    )
    .await;
    assert_eq!(channel_depth["open"], 1, "a participant counts its DM");
}

/// A claim moves a thread from ready to assigned in the workspace count, and a
/// token for another tenant is refused on both surfaces without a count.
#[tokio::test]
async fn two_tenants_count_only_their_own_work() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let ws_a = workspace(store, "tenant-a").await;
    let ws_b = workspace(store, "tenant-b").await;
    let agent_a = member(store, ws_a, "agent", MemberKind::Agent).await;
    let agent_b = member(store, ws_b, "agent", MemberKind::Agent).await;
    let chan_a = channel(store, ws_a, "work", false).await;
    let chan_b = channel(store, ws_b, "work", false).await;
    thread(store, chan_a).await;
    for _ in 0..3 {
        thread(store, chan_b).await;
    }
    let bearer_a = agent_token(store, ws_a, agent_a).await;
    let bearer_b = agent_token(store, ws_b, agent_b).await;

    let (depth, occupancy) = counts(&h, &bearer_a, ws_a).await;
    assert_eq!(depth["open"], 1, "A counts its own thread only: {depth}");
    assert_eq!(occupancy["open"], 1);
    let (depth, _) = counts(&h, &bearer_b, ws_b).await;
    assert_eq!(depth["open"], 3);

    for path in ["queue-depth", "occupancy"] {
        let (status, body) = get(&h, &bearer_a, &format!("/workspaces/{}/{path}", ws_b.0)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
        assert!(
            !body.to_string().contains("\"open\""),
            "no count leaks: {body}"
        );
    }
    for tool in ["get_workspace_queue_depth", "get_workspace_occupancy"] {
        let res = call_tool(&h, &bearer_a, tool, json!({ "workspace_id": ws_b.0 })).await;
        assert!(
            res.get("error").is_some() || res["result"]["isError"] == true,
            "{tool} refuses another workspace: {res}"
        );
        assert!(!res.to_string().contains("\\\"open\\\""), "{res}");
    }

    let res = h
        .send(
            Method::POST,
            &format!("/workspaces/{}/threads/claim-next", ws_a.0),
            &bearer_a,
            Some(json!({})),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let (depth, occupancy) = counts(&h, &bearer_a, ws_a).await;
    assert_eq!(
        (depth["ready"].as_i64(), depth["assigned"].as_i64()),
        (Some(0), Some(1))
    );
    assert_eq!(occupancy["claimed"], 1);
    let (depth, _) = counts(&h, &bearer_b, ws_b).await;
    assert_eq!(depth["ready"], 3, "B's queue did not move");
}

async fn transition(h: &Harness, bearer: &str, thread_id: ThreadId) {
    for action in ["start_review", "close"] {
        let res = h
            .send(
                Method::POST,
                &format!("/threads/{}", thread_id.0),
                bearer,
                Some(json!({ "action": action })),
            )
            .await;
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        assert_eq!(status, StatusCode::OK, "{thread_id:?} {action}: {body}");
    }
}

/// `wait_for_ready` without a channel waits on the whole workspace by the
/// claim's read rule: a thread that becomes ready first in a private channel
/// the waiter is not in, in a DM between two others, or in another workspace
/// does not wake it, and the thread it can read does. The `since_log_id`
/// replay skips the same threads.
#[tokio::test]
async fn wait_for_ready_in_the_workspace_skips_what_the_caller_may_not_read() {
    // The served `/mcp` has no event bus of its own (only stdio wires one), so
    // the waits are given the server's.
    let h = spawn_with(|state| {
        state.mcp = std::sync::Arc::new(
            maidan_mcp::McpServer::new(
                state.store.clone(),
                state.artifacts.clone(),
                state.search.clone(),
                state.embedding_provider.clone(),
            )
            .with_event_bus(state.bus.clone()),
        );
    })
    .await;
    let store = h.store.as_ref();
    let ws = workspace(store, "waits").await;
    let ws_b = workspace(store, "waits-elsewhere").await;
    let waiter = member(store, ws, "waiter", MemberKind::Agent).await;
    let insider = member(store, ws, "insider", MemberKind::Agent).await;
    let other = member(store, ws, "other", MemberKind::Human).await;
    let agent_b = member(store, ws_b, "agent", MemberKind::Agent).await;
    let open = channel(store, ws, "open", false).await;
    let secret = channel(store, ws, "secret", true).await;
    let chan_b = channel(store, ws_b, "work", false).await;
    store
        .add_channel_member(secret, insider, ChannelMemberRole::Member)
        .await
        .unwrap();
    let dm = store
        .open_dm_conversation(ws, insider, other)
        .await
        .unwrap();

    let hidden_dep = thread(store, secret).await;
    let hidden = thread(store, secret).await;
    let dm_dep = thread(store, open).await;
    let foreign_dep = thread(store, chan_b).await;
    let foreign = thread(store, chan_b).await;
    let visible_dep = thread(store, open).await;
    let visible = thread(store, open).await;
    for (dependent, dependency) in [
        (hidden, hidden_dep),
        (dm.thread_id, dm_dep),
        (foreign, foreign_dep),
        (visible, visible_dep),
    ] {
        store
            .add_thread_dependency(dependent, dependency)
            .await
            .unwrap();
    }
    let waiter_bearer = agent_token(store, ws, waiter).await;
    let insider_bearer = agent_token(store, ws, insider).await;
    let bearer_b = agent_token(store, ws_b, agent_b).await;
    let high_water = store
        .list_events_after(ws, 0, 10_000)
        .await
        .unwrap()
        .last()
        .map_or(0, |e| e.id);

    let wait = call_tool(
        &h,
        &waiter_bearer,
        "wait_for_ready",
        json!({ "timeout_ms": 5000 }),
    );
    let unblock = async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        for (bearer, dep) in [
            (&insider_bearer, hidden_dep),
            (&insider_bearer, dm_dep),
            (&bearer_b, foreign_dep),
            (&insider_bearer, visible_dep),
        ] {
            transition(&h, bearer, dep).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    };
    let (res, ()) = tokio::join!(wait, unblock);
    let event = tool_json(&res);
    assert_eq!(
        event["thread_id"],
        json!(visible.0.to_string()),
        "woken by the thread it can read, not the private, DM or foreign one: {event}"
    );

    let replayed = tool_json(
        &call_tool(
            &h,
            &waiter_bearer,
            "wait_for_ready",
            json!({ "timeout_ms": 200, "since_log_id": high_water }),
        )
        .await,
    );
    assert_eq!(
        replayed["thread_id"],
        json!(visible.0.to_string()),
        "the replay skips the same threads: {replayed}"
    );

    let insider_replay = tool_json(
        &call_tool(
            &h,
            &insider_bearer,
            "wait_for_ready",
            json!({ "timeout_ms": 200, "since_log_id": high_water }),
        )
        .await,
    );
    assert_eq!(
        insider_replay["thread_id"],
        json!(hidden.0.to_string()),
        "a member of the private channel is woken by its thread: {insider_replay}"
    );
}
