//! Writes that name another tenant's row by id from the caller's *own*
//! workspace path. `tenant_isolation_e2e` puts the victim's workspace in the
//! path, which the workspace check refuses first; here the path is the
//! caller's, so only the store's scoping stands between the call and the row.

mod seeded_workspace;

use maidan_auth::capability;
use maidan_types::{
    MemberKind, NewChannel, NewFsmHook, NewSlashCommand, NewThread, NewWorkspace, SlashHandlerKind,
    WorkspaceId,
};
use reqwest::Method;
use seeded_workspace::{member, spawn, token, Harness};
use serde_json::{json, Value};

struct Tenant {
    ws: WorkspaceId,
    bearer: String,
    thread: uuid::Uuid,
}

async fn tenant(h: &Harness, name: &str) -> Tenant {
    let store = h.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let m = member(store, ws, &format!("{name}-admin"), MemberKind::Human).await;
    let bearer = token(store, ws, m, capability::all()).await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    Tenant {
        ws,
        bearer,
        thread: thread.id.0,
    }
}

async fn mcp_call(h: &Harness, bearer: &str, name: &str, arguments: Value) -> Value {
    h.send(
        Method::POST,
        "/mcp",
        bearer,
        Some(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })),
    )
    .await
    .json()
    .await
    .unwrap()
}

#[tokio::test]
async fn another_tenants_admin_cannot_revoke_by_id_via_rest_or_mcp() {
    let h = spawn().await;
    let a = tenant(&h, "tenant-a").await;
    let b = tenant(&h, "tenant-b").await;
    let command = h
        .store
        .create_slash_command(NewSlashCommand {
            workspace_id: a.ws,
            name: "ping".into(),
            description: None,
            handler_kind: SlashHandlerKind::McpTool,
            handler_target: "list_channels".into(),
            secret_ciphertext: String::new(),
            created_by_token: None,
        })
        .await
        .unwrap();
    let hook = h
        .store
        .create_fsm_hook(NewFsmHook {
            workspace_id: a.ws,
            label: None,
            from_state: None,
            to_state: None,
            handler_kind: SlashHandlerKind::McpTool,
            handler_target: "list_channels".into(),
            secret_ciphertext: String::new(),
            created_by_token: None,
        })
        .await
        .unwrap();

    let res = h
        .send(
            Method::DELETE,
            &format!("/workspaces/{}/slash-commands/{}", b.ws.0, command.id.0),
            &b.bearer,
            None,
        )
        .await;
    assert_eq!(res.status(), 404);
    let res = h
        .send(
            Method::DELETE,
            &format!("/workspaces/{}/fsm-hooks/{}", b.ws.0, hook.id.0),
            &b.bearer,
            None,
        )
        .await;
    assert_eq!(res.status(), 404);
    let res = mcp_call(
        &h,
        &b.bearer,
        "revoke_slash_command",
        json!({ "workspace_id": b.ws.0, "command_id": command.id.0 }),
    )
    .await;
    assert_eq!(res["error"]["code"].as_i64(), Some(-32004), "{res}");
    let res = mcp_call(
        &h,
        &b.bearer,
        "revoke_fsm_hook",
        json!({ "workspace_id": b.ws.0, "hook_id": hook.id.0 }),
    )
    .await;
    assert_eq!(res["error"]["code"].as_i64(), Some(-32004), "{res}");

    assert!(
        h.store
            .get_slash_command(command.id)
            .await
            .unwrap()
            .command
            .enabled,
        "A's command is still active"
    );
    assert!(
        h.store.get_fsm_hook(hook.id).await.unwrap().hook.enabled,
        "A's hook is still active"
    );

    let res = h
        .send(
            Method::DELETE,
            &format!("/workspaces/{}/slash-commands/{}", a.ws.0, command.id.0),
            &a.bearer,
            None,
        )
        .await;
    assert_eq!(res.status(), 204, "the owner can still revoke");
}

#[tokio::test]
async fn two_tenants_link_the_same_slack_channel_and_github_issue_each_for_itself() {
    let h = spawn().await;
    let a = tenant(&h, "tenant-a").await;
    let b = tenant(&h, "tenant-b").await;
    let slack = |t: &Tenant| json!({ "slack_channel_id": "CX1", "thread_id": t.thread });
    let github = |t: &Tenant| json!({ "repo": "o/r", "issue_number": 7, "thread_id": t.thread });

    for (route, body) in [("slack-links", slack(&a)), ("github-links", github(&a))] {
        let res = h
            .send(
                Method::POST,
                &format!("/workspaces/{}/{route}", a.ws.0),
                &a.bearer,
                Some(body.clone()),
            )
            .await;
        assert_eq!(res.status(), 201, "{route}");
        let res = h
            .send(
                Method::POST,
                &format!("/workspaces/{}/{route}", a.ws.0),
                &a.bearer,
                Some(body),
            )
            .await;
        assert_eq!(res.status(), 201, "{route}: A may re-link its own");
    }
    // B links the same channel and issue: it neither takes A's nor is told A
    // has them.
    for (route, body) in [("slack-links", slack(&b)), ("github-links", github(&b))] {
        let res = h
            .send(
                Method::POST,
                &format!("/workspaces/{}/{route}", b.ws.0),
                &b.bearer,
                Some(body),
            )
            .await;
        assert_eq!(res.status(), 201, "{route}");
    }

    let slack_link = h
        .store
        .get_slack_channel_link(a.ws, "CX1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(slack_link.thread_id.0, a.thread);
    let gh_link = h
        .store
        .get_github_issue_link(a.ws, "o/r", 7)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(gh_link.thread_id.0, a.thread);
    let theirs = h
        .store
        .get_slack_channel_link(b.ws, "CX1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(theirs.thread_id.0, b.thread);

    // One tenant's unlink leaves the other's link in place.
    let res = h
        .send(
            Method::DELETE,
            &format!("/workspaces/{}/slack-links/CX1", a.ws.0),
            &a.bearer,
            None,
        )
        .await;
    assert_eq!(res.status(), 204);
    assert!(h
        .store
        .get_slack_channel_link(b.ws, "CX1")
        .await
        .unwrap()
        .is_some());
}
