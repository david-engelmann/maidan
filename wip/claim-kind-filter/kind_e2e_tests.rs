
/// A thread in `channel_id` described as `kind`, with no footprint and no
/// engine.
async fn kind_task(store: &dyn Store, channel_id: ChannelId, kind: maidan_types::TaskKind) -> ThreadId {
    store
        .create_thread_with_footprint(
            NewThread {
                channel_id,
                parent_thread_id: None,
                title: None,
                description: None,
                created_by: None,
            },
            None,
            Some(maidan_types::TaskAttributes {
                kind,
                ..maidan_types::TaskAttributes::default()
            }),
            None,
        )
        .await
        .unwrap()
        .0
        .id
}

/// `kind` narrows the workspace depth and claim-next on both surfaces, and a
/// thread that never named its kind counts as a change.
#[tokio::test]
async fn the_workspace_depth_and_claim_next_narrow_by_kind_on_both_surfaces() {
    use maidan_types::TaskKind::{Change, Review};
    let h = spawn().await;
    let store = h.store.as_ref();
    let ws = workspace(store, "kinds").await;
    let agent = member(store, ws, "agent", MemberKind::Agent).await;
    let chan = channel(store, ws, "work", false).await;
    let first = kind_task(store, chan, Review).await;
    let second = kind_task(store, chan, Review).await;
    kind_task(store, chan, Change).await;
    thread(store, chan).await;
    let bearer = agent_token(store, ws, agent).await;

    let reviews =
        filtered_depth(&h, &bearer, ws, "kind=review", json!({ "kind": "review" })).await;
    assert_eq!(reviews["ready"], 2, "the two reviews only: {reviews}");
    let changes =
        filtered_depth(&h, &bearer, ws, "kind=change", json!({ "kind": "change" })).await;
    assert_eq!(
        changes["ready"], 2,
        "the change and the thread that never named a kind: {changes}"
    );
    let plans = filtered_depth(&h, &bearer, ws, "kind=plan", json!({ "kind": "plan" })).await;
    assert_eq!(plans["ready"], 0, "{plans}");

    let res = h
        .send(
            Method::POST,
            &format!("/workspaces/{}/threads/claim-next", ws.0),
            &bearer,
            Some(json!({ "kind": "review" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let claimed: Value = res.json().await.unwrap();
    assert_eq!(claimed["id"], json!(first.0.to_string()), "{claimed}");
    let claimed = tool_json(
        &call_tool(
            &h,
            &bearer,
            "claim_next_workspace_thread",
            json!({ "workspace_id": ws.0, "kind": "review" }),
        )
        .await,
    );
    assert_eq!(claimed["id"], json!(second.0.to_string()), "{claimed}");
    let nothing = tool_json(
        &call_tool(
            &h,
            &bearer,
            "claim_next_workspace_thread",
            json!({ "workspace_id": ws.0, "kind": "review" }),
        )
        .await,
    );
    assert_eq!(nothing, Value::Null, "no change is handed to a review claim");
}

/// An unknown `kind` is refused with a 400 over REST and invalid params over
/// MCP, on claim-next and on the workspace depth, each naming the value and
/// the kinds there are.
#[tokio::test]
async fn an_unknown_kind_is_refused_with_a_clear_message_over_rest_and_mcp() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let ws = workspace(store, "bad-kind").await;
    let agent = member(store, ws, "agent", MemberKind::Agent).await;
    let chan = channel(store, ws, "work", false).await;
    thread(store, chan).await;
    let bearer = agent_token(store, ws, agent).await;
    let names_the_kinds = |detail: &str| {
        detail.contains("unknown variant `bogus`")
            && detail.contains("`change`")
            && detail.contains("`plan`")
            && detail.contains("`review`")
    };

    for path in [
        format!("/workspaces/{}/threads/claim-next", ws.0),
        format!("/channels/{}/threads/claim-next", chan.0),
    ] {
        let res = h
            .send(Method::POST, &path, &bearer, Some(json!({ "kind": "bogus" })))
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{path}");
        let body: Value = res.json().await.unwrap();
        assert!(names_the_kinds(body["detail"].as_str().unwrap_or_default()), "{path}: {body}");
    }
    let (status, body) = get(
        &h,
        &bearer,
        &format!("/workspaces/{}/queue-depth?kind=bogus", ws.0),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(names_the_kinds(body["detail"].as_str().unwrap_or_default()), "{body}");

    for (tool, args) in [
        (
            "claim_next_workspace_thread",
            json!({ "workspace_id": ws.0, "kind": "bogus" }),
        ),
        (
            "claim_next_thread",
            json!({ "channel_id": chan.0, "kind": "bogus" }),
        ),
        (
            "get_workspace_queue_depth",
            json!({ "workspace_id": ws.0, "kind": "bogus" }),
        ),
    ] {
        let res = call_tool(&h, &bearer, tool, args).await;
        assert_eq!(res["error"]["code"], -32602, "{tool}: {res}");
        let message = res["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("kind") && names_the_kinds(message),
            "{tool}: {res}"
        );
    }
    let (_, depth) = get(&h, &bearer, &format!("/workspaces/{}/queue-depth", ws.0)).await;
    assert_eq!(depth["assigned"], 0, "nothing was claimed: {depth}");
}
