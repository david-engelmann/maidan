//! Tenant isolation, as a conformance check over the whole HTTP surface.
//!
//! Workspace B holds a private channel, a thread, a message and a second
//! member. Workspace A's token carries every workspace-scoped capability.
//! Every operation in the served OpenAPI document whose path names one of B's
//! entities is called with A's token and B's ids, and each must refuse: no
//! 2xx, no B content in the body. B's entities must be intact afterwards.
//!
//! A new route is covered by being in the spec; nothing needs registering
//! here. A path parameter this file does not know gets a random id, which
//! proves nothing, so the known set is asserted to stay large.

mod seeded_workspace;

use std::{collections::BTreeMap, time::Duration};

use maidan_auth::capability;
use maidan_types::{MemberKind, NewWorkspace};
use reqwest::Method;
use seeded_workspace::{
    example, fill, json_body_schema, member, query, seed_victim, spawn, token, Harness, Victim,
    SECRET_BODY, SECRET_CHANNEL,
};
use serde_json::{json, Value};

/// Every capability that is scoped to the token's own workspace. The global
/// ones (`audit:read-global`, `operator:global`) and federation's peer
/// capabilities cross workspaces by design and are left out.
fn workspace_scoped_capabilities() -> Vec<String> {
    capability::all()
        .into_iter()
        .filter(|c| {
            ![
                capability::AUDIT_READ_GLOBAL,
                capability::OPERATOR_GLOBAL,
                capability::FEDERATION_INGEST,
                capability::FEDERATION_ADMIN,
            ]
            .contains(&c.as_str())
        })
        .collect()
}

async fn attacker(h: &Harness) -> String {
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "tenant-a".into(),
        })
        .await
        .unwrap()
        .id;
    let m = member(h.store.as_ref(), ws, "a-agent", MemberKind::Agent).await;
    token(h.store.as_ref(), ws, m, workspace_scoped_capabilities()).await
}

fn leaked(text: &str) -> bool {
    text.contains(SECRET_BODY) || text.contains(SECRET_CHANNEL)
}

#[tokio::test]
async fn no_http_operation_serves_or_changes_another_workspace() {
    let h = spawn().await;
    let doc = serde_json::to_value(maidan_server::openapi::document()).unwrap();
    let victim = seed_victim(&h, &doc).await;
    let a = attacker(&h).await;

    let mut leaks = Vec::new();
    let mut probed = 0;
    let mut unseeded = BTreeMap::<String, usize>::new();
    let mut body_refused = Vec::new();
    for (template, item) in doc["paths"].as_object().unwrap() {
        if !template.contains('{') {
            continue;
        }
        let Some(path) = fill(template, &victim) else {
            let seg = template
                .split('/')
                .collect::<Vec<_>>()
                .windows(2)
                .find(|w| w[1].starts_with('{') && !victim.ids.contains_key(w[0]))
                .map(|w| format!("{}/{}", w[0], w[1]))
                .unwrap_or_default();
            *unseeded.entry(seg).or_default() += 1;
            continue;
        };
        for (method, op) in item.as_object().unwrap() {
            let Ok(method) = method.to_uppercase().parse::<Method>() else {
                continue;
            };
            if !matches!(
                method,
                Method::GET | Method::POST | Method::PUT | Method::PATCH | Method::DELETE
            ) {
                continue;
            }
            probed += 1;
            let body = match json_body_schema(op) {
                Some(schema) => Some(example(&doc, schema, "", &victim, 0)),
                None => {
                    matches!(method, Method::POST | Method::PUT | Method::PATCH).then(|| json!({}))
                }
            };
            let body = body.map(|mut b| {
                // The one closed set whose values differ by route.
                if template.contains("approval-gates") && b.get("action").is_some() {
                    b["action"] = json!("accept");
                }
                b
            });
            let path = format!("{path}{}", query(&doc, op, &victim));
            let res = h.send(method.clone(), &path, &a, body).await;
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            let label = format!("{method} {template}");
            if status.is_success() || status.is_redirection() {
                leaks.push(format!("{label}: {status}"));
            } else if status.as_u16() == 429 {
                leaks.push(format!(
                    "{label}: rate limited, so the check proved nothing"
                ));
            } else if leaked(&text) {
                leaks.push(format!(
                    "{label}: {status} with tenant B's content in the body"
                ));
            } else if matches!(status.as_u16(), 400 | 413 | 415 | 422) {
                body_refused.push(format!(
                    "{label}: {status} {}",
                    text.chars().take(160).collect::<String>()
                ));
            }
        }
    }
    println!("probed {probed}; not seeded: {unseeded:?}");
    assert!(
        leaks.is_empty(),
        "workspace A reached workspace B:\n{}",
        leaks.join("\n")
    );
    // A refusal at the body says nothing about access; every body is built
    // to pass validation, so none should be refused there.
    assert!(
        body_refused.is_empty(),
        "refused before any access check:\n{}",
        body_refused.join("\n")
    );
    assert!(probed >= 260, "only {probed} operations probed");
    // The kinds left unseeded, each for a reason: dead letters are
    // instance-wide and `operator:global`; multipart needs the S3 backend;
    // quarantined outbox rows are only made by a failing relay.
    let allowed = ["dead/{id}", "multipart/{upload_id}", "outbox/{oid}"];
    for kind in unseeded.keys() {
        assert!(
            allowed.contains(&kind.as_str()),
            "no victim id for {kind}: seed it"
        );
    }
    assert_victim_intact(&h, &victim).await;
}

/// Nothing workspace A sent changed workspace B.
async fn assert_victim_intact(h: &Harness, victim: &Victim) {
    let msg: Value = h
        .send(
            Method::GET,
            &format!("/messages/{}", victim.ids["messages"]),
            &victim.bearer,
            None,
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(msg["body"], SECRET_BODY, "B's message changed: {msg}");
    for kind in ["threads", "channels", "workspaces"] {
        let path = format!("/{kind}/{}", victim.ids[kind]);
        let res = h.send(Method::GET, &path, &victim.bearer, None).await;
        assert!(res.status().is_success(), "{path}: {}", res.status());
    }
    let members: Value = h
        .send(
            Method::GET,
            &format!("/workspaces/{}/members", victim.ids["workspaces"]),
            &victim.bearer,
            None,
        )
        .await
        .json()
        .await
        .unwrap();
    assert!(
        members.to_string().contains(&victim.ids["reviewers"]),
        "B's member is gone: {members}"
    );
}

async fn mcp(h: &Harness, bearer: &str, method: &str, params: Value) -> Value {
    h.send(
        Method::POST,
        "/mcp",
        bearer,
        Some(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })),
    )
    .await
    .json()
    .await
    .unwrap()
}

/// Whether any string in `args` is one of the victim's ids.
fn names_victim(args: &Value, victim: &Victim) -> bool {
    match args {
        Value::String(s) => victim.ids.values().any(|id| id == s),
        Value::Array(a) => a.iter().any(|v| names_victim(v, victim)),
        Value::Object(o) => o.values().any(|v| names_victim(v, victim)),
        _ => false,
    }
}

#[tokio::test]
async fn no_mcp_tool_serves_or_changes_another_workspace() {
    let h = spawn().await;
    let doc = serde_json::to_value(maidan_server::openapi::document()).unwrap();
    let victim = seed_victim(&h, &doc).await;
    let a = attacker(&h).await;

    let listed = mcp(&h, &a, "tools/list", json!({})).await;
    let tools = listed["result"]["tools"]
        .as_array()
        .expect("tools/list")
        .clone();
    let mut leaks = Vec::new();
    let mut probed = 0;
    let mut invalid = Vec::new();
    for tool in &tools {
        let name = tool["name"].as_str().unwrap();
        let args = example(&doc, &tool["inputSchema"], "", &victim, 0);
        if !names_victim(&args, &victim) {
            continue;
        }
        probed += 1;
        let res = mcp(
            &h,
            &a,
            "tools/call",
            json!({ "name": name, "arguments": args }),
        )
        .await;
        let text = res.to_string();
        let refused = res.get("error").is_some() || res["result"]["isError"] == true
            // Answers another workspace's link exactly as it answers no link.
            || (name == "unlink_slack_channel" && text.contains(r#"\"unlinked\":false"#));
        if !refused {
            leaks.push(format!(
                "{name}: answered {}",
                text.chars().take(200).collect::<String>()
            ));
        } else if leaked(&text) {
            leaks.push(format!("{name}: refused with tenant B's content"));
        } else if res["error"]["code"] == -32602
            && [
                "missing field",
                "unknown field",
                "invalid type",
                "unknown variant",
                "invalid value",
                "expected",
            ]
            .iter()
            .any(|m| res["error"]["message"].as_str().unwrap_or("").contains(m))
        {
            // The arguments did not decode, so no access check ran.
            invalid.push(format!("{name}: {}", res["error"]["message"]));
        }
    }
    // The streams are not OpenAPI operations, so the router source is the
    // list. A new `.route` whose path contains `stream` or `subscribe` fails
    // here until `stream_probes` covers it.
    assert_eq!(
        stream_routes(),
        vec![
            "/agui/stream",
            "/mcp/notifications",
            "/mcp/stream",
            "/mcp/streamable",
            "/ws/subscribe"
        ],
        "a live stream was added or renamed: tenant_isolation_e2e must probe it"
    );
    println!(
        "probed {probed} of {} tools; victim workspace {}",
        tools.len(),
        victim.ids["workspaces"]
    );
    assert!(
        leaks.is_empty(),
        "workspace A reached workspace B over MCP:\n{}",
        leaks.join("\n")
    );
    assert!(
        invalid.is_empty(),
        "refused as invalid before any access check:\n{}",
        invalid.join("\n")
    );
    assert!(probed >= 50, "only {probed} tools probed");
    assert_victim_intact(&h, &victim).await;
}

/// The live-stream routes, read from the router in `app.rs` (the streams are
/// not OpenAPI operations, so the spec cannot drive this). A route registered
/// with `.route(...)` whose path contains `stream` or `subscribe` is one, and
/// the test asserts the exact set, so a new stream fails it until it is probed.
/// The live streams the suite probes, read from the router in `app.rs`.
/// The streams are not OpenAPI operations, so the spec cannot drive this, and
/// a keyword scan of the paths cannot either: `/mcp/notifications` is a
/// stream whose path says neither. So the probed set is asserted exactly, and
/// the total `.route(` count is pinned: adding or removing a route anywhere
/// fails the test, which is the prompt to check whether the new route is a
/// stream that needs a probe here.
fn stream_routes() -> Vec<String> {
    let source = include_str!("../src/app.rs");
    let route_count = source.matches(".route(").count();
    assert_eq!(
        route_count, 320,
        "{route_count} routes in app.rs; if one you added is a live stream, probe it in stream_probes and update both numbers"
    );
    let mut routes = std::collections::BTreeSet::new();
    let mut rest = source;
    while let Some(start) = rest.find(".route(") {
        rest = &rest[start + ".route(".len()..];
        let trimmed = rest.trim_start();
        if !trimmed.starts_with('"') {
            continue;
        }
        let after = &trimmed[1..];
        let Some(end) = after.find('"') else {
            continue;
        };
        let path = &after[..end];
        if matches!(
            path,
            "/ws/subscribe"
                | "/mcp/stream"
                | "/mcp/streamable"
                | "/agui/stream"
                | "/mcp/notifications"
        ) {
            routes.insert(path.to_string());
        }
    }
    routes.into_iter().collect()
}

/// One probe per way a caller can name the victim on each stream route.
fn stream_probes(ws: &str, channel: &str, thread: &str) -> Vec<String> {
    stream_routes()
        .into_iter()
        .flat_map(|path| match path.as_str() {
            // Probed over the WebSocket, and the protocol transports
            // (streamable HTTP, resource notifications), which carry no
            // workspace filter to name a victim by.
            "/ws/subscribe" | "/mcp/streamable" | "/mcp/notifications" => vec![],
            "/agui/stream" => ["workspace_id", "thread_id", "channel_id"]
                .into_iter()
                .map(|param| {
                    let value = match param {
                        "workspace_id" => ws,
                        "thread_id" => thread,
                        "channel_id" => channel,
                        _ => unreachable!(),
                    };
                    format!("{path}?{param}={value}")
                })
                .collect(),
            _ => vec![format!("{path}?workspace_id={ws}")],
        })
        .collect()
}

/// Everything an open SSE response sends within `window`.
async fn drain_sse(res: reqwest::Response, window: Duration) -> String {
    use futures::StreamExt;
    let mut body = res.bytes_stream();
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(Ok(chunk))) = tokio::time::timeout_at(deadline, body.next()).await {
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    seen
}

#[tokio::test]
async fn no_live_stream_carries_another_workspaces_events() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let h = spawn().await;
    let doc = serde_json::to_value(maidan_server::openapi::document()).unwrap();
    let victim = seed_victim(&h, &doc).await;
    let a = attacker(&h).await;
    let (ws, channel, thread) = (
        &victim.ids["workspaces"],
        &victim.ids["channels"],
        &victim.ids["threads"],
    );

    // Every way a subscriber can name the victim, on every transport.
    let filters = [
        json!({ "workspace_id": ws }),
        json!({ "workspace_id": ws, "channel_id": channel }),
        json!({ "workspace_id": ws, "thread_id": thread }),
        json!({ "channel_id": channel }),
        json!({ "thread_id": thread }),
        json!({ "dm_conversation_id": victim.ids["dm"] }),
        json!({ "member_id": victim.ids["members"] }),
    ];
    let mut sockets = Vec::new();
    for filter in &filters {
        let (mut sock, _) = connect_async(format!("ws://{}/ws/subscribe", h.addr))
            .await
            .unwrap();
        sock.send(Message::Text(
            json!({ "token": a, "filter": filter }).to_string(),
        ))
        .await
        .unwrap();
        sockets.push((filter.clone(), sock));
    }
    // The control: the victim's own subscription must see what A must not,
    // or the window proves nothing.
    let (mut control, _) = connect_async(format!("ws://{}/ws/subscribe", h.addr))
        .await
        .unwrap();
    control
        .send(Message::Text(
            json!({ "token": victim.bearer, "filter": { "workspace_id": ws } }).to_string(),
        ))
        .await
        .unwrap();
    let queries = stream_probes(ws, channel, thread);
    // Each SSE response is drained from the moment it opens, concurrently,
    // over a window that covers the victim's writes; the request timeout is
    // longer than the window so it cannot cut a drain short. The victim's own
    // SSE streams are the control for this transport.
    const SSE_WINDOW: Duration = Duration::from_secs(3);
    let mut streams = Vec::new();
    for (who, token) in [("attacker", &a), ("victim", &victim.bearer)] {
        for q in &queries {
            let res = h
                .client
                .get(h.url(q))
                .header("Authorization", format!("Bearer {token}"))
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .unwrap();
            let ok = res.status().is_success();
            let drain = tokio::spawn(async move {
                if ok {
                    drain_sse(res, SSE_WINDOW).await
                } else {
                    String::new()
                }
            });
            streams.push((who, q.clone(), drain));
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The victim acts: a new channel (its name is in the event), then a
    // thread message and a DM (their words are sealed in the log, but a
    // regression could carry them).
    let created = h
        .send(
            Method::POST,
            &format!("/workspaces/{ws}/channels"),
            &victim.bearer,
            Some(json!({ "name": format!("{SECRET_CHANNEL}-live") })),
        )
        .await;
    assert!(created.status().is_success(), "{}", created.status());
    let posted = h
        .send(
            Method::POST,
            &format!("/threads/{thread}/messages"),
            &victim.bearer,
            Some(json!({ "body": format!("{SECRET_BODY} live") })),
        )
        .await;
    assert!(posted.status().is_success(), "{}", posted.status());
    let dm = h
        .send(
            Method::POST,
            &format!("/dm/{}/messages", victim.ids["dm"]),
            &victim.bearer,
            Some(json!({ "body": format!("{SECRET_BODY} dm") })),
        )
        .await;
    assert!(dm.status().is_success(), "{}", dm.status());

    let mut control_saw = false;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(800);
    while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, control.next()).await {
        if let Message::Text(text) = msg {
            control_saw |= leaked(&text);
        }
    }
    assert!(
        control_saw,
        "the victim's own subscription saw nothing, so the window proves nothing"
    );

    let mut leaks = Vec::new();
    for (filter, mut sock) in sockets {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(800);
        while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, sock.next()).await {
            if let Message::Text(text) = msg {
                if leaked(&text) {
                    leaks.push(format!("/ws/subscribe {filter}: {text}"));
                }
            }
        }
    }
    let mut sse_control_saw = Vec::new();
    for (who, q, drain) in streams {
        let seen = drain.await.unwrap();
        if !leaked(&seen) {
            continue;
        }
        if who == "victim" {
            sse_control_saw.push(q);
        } else {
            leaks.push(format!(
                "{q}: {}",
                seen.chars().take(300).collect::<String>()
            ));
        }
    }
    assert_eq!(
        sse_control_saw.len(),
        queries.len(),
        "each victim SSE control must see its events, or that stream's window proves nothing; saw: {sse_control_saw:?}"
    );
    assert!(
        leaks.is_empty(),
        "another workspace's events reached workspace A:\n{}",
        leaks.join("\n")
    );
}
