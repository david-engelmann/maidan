//! `GET /tools` is the caller's `tools/list`, over REST. Both read
//! `maidan_mcp::tools::catalog_for`, and this pins that they stay one list:
//! a capability-scoped token gets the same tools, with the same schemas, from
//! each surface.

mod seeded_workspace;

use std::collections::BTreeMap;

use maidan_auth::capability;
use maidan_types::{MemberKind, NewWorkspace};
use reqwest::{Method, StatusCode};
use seeded_workspace::{member, spawn, token};
use serde_json::{json, Value};

fn by_name(tools: &Value) -> BTreeMap<String, Value> {
    tools
        .as_array()
        .expect("a tools array")
        .iter()
        .map(|t| {
            (
                t["name"].as_str().expect("a tool name").to_string(),
                t.clone(),
            )
        })
        .collect()
}

#[tokio::test]
async fn the_rest_tool_catalog_is_the_mcp_tools_list() {
    let h = spawn().await;
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "tools".into(),
        })
        .await
        .unwrap()
        .id;
    let m = member(h.store.as_ref(), ws, "reader", MemberKind::Agent).await;
    // A narrow token, so the parity covers the capability filter too.
    let caps = vec![
        capability::WORKSPACE_READ.to_string(),
        capability::SEARCH_QUERY.to_string(),
    ];
    let bearer = token(h.store.as_ref(), ws, m, caps).await;

    let rest = h.send(Method::GET, "/tools", &bearer, None).await;
    assert_eq!(rest.status(), StatusCode::OK);
    let rest: Value = rest.json().await.unwrap();

    let listed: Value = h
        .send(
            Method::POST,
            "/mcp",
            &bearer,
            Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} })),
        )
        .await
        .json()
        .await
        .unwrap();

    let rest = by_name(&rest["tools"]);
    let mcp = by_name(&listed["result"]["tools"]);
    assert!(!rest.is_empty(), "a workspace:read token lists some tools");
    assert!(
        rest.len() < maidan_mcp::tools::catalog().len(),
        "the narrow token sees a filtered catalog, not every tool"
    );
    assert_eq!(
        rest.keys().collect::<Vec<_>>(),
        mcp.keys().collect::<Vec<_>>(),
        "GET /tools and tools/list name the same tools"
    );
    for (name, tool) in &rest {
        for field in ["title", "description", "inputSchema", "annotations"] {
            assert_eq!(
                tool[field], mcp[name][field],
                "{name}.{field} differs between GET /tools and tools/list"
            );
        }
    }

    let refused = h.send(Method::GET, "/tools", "not-a-token", None).await;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
}
