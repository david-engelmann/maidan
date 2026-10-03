mod seeded_workspace;
use maidan_auth::capability;
use maidan_types::{MemberKind, NewWorkspace};
use reqwest::Method;
use seeded_workspace::{member, spawn, token};
use serde_json::{json, Value};

#[tokio::test]
async fn probe_wait_over_http() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let ws = store.create_workspace(NewWorkspace { name: "p".into() }).await.unwrap().id;
    let agent = member(store, ws, "agent", MemberKind::Agent).await;
    let bearer = token(store, ws, agent, vec![capability::WORKSPACE_READ.into()]).await;
    let res: Value = h.send(Method::POST, "/mcp", &bearer, Some(json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"wait_for_ready","arguments":{"timeout_ms":200}}
    }))).await.json().await.unwrap();
    eprintln!("RESULT: {res}");
}
