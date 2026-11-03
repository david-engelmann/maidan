//! `GET /tools` is the caller's `tools/list`, over REST. Both read
//! `maidan_mcp::tools::catalog_for`, and this pins that they stay one list:
//! a capability-scoped token gets the same tools, with the same schemas, from
//! each surface. The console's session-authed `GET /ui/api/tools` serves the
//! same handler, so a signed-in person reads that one list too.

mod seeded_workspace;

use std::collections::BTreeMap;

use maidan_auth::capability;
use maidan_types::{MemberKind, NewMaidanSession, NewWorkspace};
use reqwest::{Method, StatusCode};
use seeded_workspace::{member, spawn, spawn_with, token};
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

const SESSION_SECRET: &[u8] = b"tool-catalog-e2e-session-secret-32b!";

#[tokio::test]
async fn the_console_session_reads_the_same_tool_catalog() {
    let h = spawn_with(|state| {
        state.sessions = Some(maidan_server::session::SessionSettings {
            secret: std::sync::Arc::from(SESSION_SECRET),
            ttl_secs: 3600,
            cookie_secure: false,
        });
    })
    .await;
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "tools-ui".into(),
        })
        .await
        .unwrap()
        .id;
    let m = member(h.store.as_ref(), ws, "person", MemberKind::Human).await;
    let caps = vec![
        capability::WORKSPACE_READ.to_string(),
        capability::SEARCH_QUERY.to_string(),
    ];
    let bearer = token(h.store.as_ref(), ws, m, caps).await;

    // The bearer gets one list from both paths: one handler, no second copy.
    let rest: Value = h
        .send(Method::GET, "/tools", &bearer, None)
        .await
        .json()
        .await
        .unwrap();
    let ui = h.send(Method::GET, "/ui/api/tools", &bearer, None).await;
    assert_eq!(ui.status(), StatusCode::OK);
    let ui: Value = ui.json().await.unwrap();
    assert_eq!(
        by_name(&rest["tools"]),
        by_name(&ui["tools"]),
        "GET /ui/api/tools is GET /tools"
    );

    // A signed-in session with no token behind it, as after an OIDC sign-in.
    let session = h
        .store
        .create_session(NewMaidanSession {
            workspace_id: ws,
            member_id: m,
            api_token_id: None,
            oidc_identity_id: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();
    let mut headers = axum::http::HeaderMap::new();
    maidan_server::session::set_session_cookie(
        &mut headers,
        session.id,
        3600,
        false,
        SESSION_SECRET,
    )
    .unwrap();
    let cookie = headers
        .get(axum::http::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let signed_in = h
        .client
        .get(h.url("/ui/api/tools"))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(signed_in.status(), StatusCode::OK);
    let signed_in: Value = signed_in.json().await.unwrap();
    let signed_in = by_name(&signed_in["tools"]);
    assert!(!signed_in.is_empty(), "a signed-in member lists some tools");
    let registry = by_name(&Value::Array(maidan_mcp::tools::catalog()));
    for (name, tool) in &signed_in {
        for field in ["title", "description", "inputSchema", "annotations"] {
            assert_eq!(
                tool[field], registry[name][field],
                "{name}.{field} differs between GET /ui/api/tools and the registry"
            );
        }
    }

    let anonymous = h.client.get(h.url("/ui/api/tools")).send().await.unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}
