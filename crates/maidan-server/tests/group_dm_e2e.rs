//! Multi-member group DM threads.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::StatusCode;
use serde_json::json;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    Arc<dyn Store>,
) {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
    let bus = Arc::new(InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), server, store)
}

async fn mint_token(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    member_id: maidan_types::MemberId,
) -> String {
    mint_token_with(
        store,
        workspace_id,
        member_id,
        &[
            capability::WORKSPACE_READ,
            capability::WORKSPACE_WRITE,
            capability::MESSAGE_POST,
        ],
    )
    .await
}

async fn mint_token_with(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    member_id: maidan_types::MemberId,
    caps: &[&str],
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn group_dm_open_post_and_list() {
    let (addr, client, server, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "group-dm".into(),
        })
        .await
        .unwrap();
    let a = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();
    let b = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "b".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();
    let c = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "c".into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap();
    let token = mint_token(store.as_ref(), ws.id, a.id).await;

    let open = client
        .post(format!("{base}/workspaces/{}/group-dms", ws.id))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "member_ids": [a.id.0, b.id.0, c.id.0],
            "title": "trio"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(open.status(), StatusCode::CREATED);
    let group: serde_json::Value = open.json().await.unwrap();
    assert_eq!(group["member_ids"].as_array().unwrap().len(), 3);
    let gid = group["id"].as_str().unwrap();

    let msg = client
        .post(format!("{base}/group-dms/{gid}/messages"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "body": "hello group" }))
        .send()
        .await
        .unwrap();
    assert_eq!(msg.status(), StatusCode::CREATED);

    let list = client
        .get(format!(
            "{base}/workspaces/{}/group-dms?member_id={}",
            ws.id, a.id.0
        ))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let rows: Vec<serde_json::Value> = list.json().await.unwrap();
    assert_eq!(rows.len(), 1);

    server.abort();
}

/// MCP-003: opening a group DM puts people in a conversation, so it needs
/// `message:post`. A read-only token is refused and writes nothing, and a
/// poster from another workspace cannot open one here.
#[tokio::test]
async fn a_read_only_token_cannot_open_a_group_dm() {
    let (addr, client, server, store) = spawn().await;
    let base = format!("http://{addr}");
    let mut members = Vec::new();
    let mut workspaces = Vec::new();
    for name in ["here", "elsewhere"] {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        for handle in ["a", "b", "c"] {
            members.push(
                store
                    .create_member(NewMember {
                        workspace_id: ws.id,
                        handle: handle.into(),
                        display_name: None,
                        kind: maidan_types::MemberKind::Human,
                    })
                    .await
                    .unwrap()
                    .id,
            );
        }
        workspaces.push(ws.id);
    }
    let (here, elsewhere) = (workspaces[0], workspaces[1]);
    let trio = json!({ "member_ids": [members[0].0, members[1].0, members[2].0] });
    let open = |token: String, ws: WorkspaceId| {
        let client = client.clone();
        let base = base.clone();
        let trio = trio.clone();
        async move {
            client
                .post(format!("{base}/workspaces/{}/group-dms", ws.0))
                .bearer_auth(token)
                .json(&trio)
                .send()
                .await
                .unwrap()
                .status()
        }
    };

    let read_only = mint_token_with(
        store.as_ref(),
        here,
        members[0],
        &[capability::WORKSPACE_READ],
    )
    .await;
    let writer = mint_token_with(
        store.as_ref(),
        here,
        members[0],
        &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE],
    )
    .await;
    assert_eq!(open(read_only, here).await, StatusCode::FORBIDDEN);
    assert_eq!(
        open(writer, here).await,
        StatusCode::FORBIDDEN,
        "workspace:write is not message:post"
    );
    let outsider = mint_token(store.as_ref(), elsewhere, members[3]).await;
    let crossed = open(outsider, here).await;
    assert!(
        matches!(crossed, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "another workspace's poster opened a group DM here: {crossed}"
    );
    for (ws, member) in [(here, members[0]), (elsewhere, members[3])] {
        assert!(
            store
                .list_group_dm_conversations_for_member(ws, member)
                .await
                .unwrap()
                .is_empty(),
            "a refused open wrote a group DM"
        );
    }

    let poster = mint_token(store.as_ref(), here, members[0]).await;
    assert_eq!(open(poster, here).await, StatusCode::CREATED);
    server.abort();
}

async fn member(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    handle: &str,
) -> maidan_types::MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: maidan_types::MemberKind::Human,
        })
        .await
        .unwrap()
        .id
}

/// The opener of a group DM is one of its members, as a 1:1 DM's opener is:
/// a list that leaves the caller out gets them added, so a token can never
/// open a conversation among others that it cannot post into. A member of
/// another workspace still cannot open one here.
#[tokio::test]
async fn group_dm_opener_is_always_a_member() {
    let (addr, client, server, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "group-dm-opener".into(),
        })
        .await
        .unwrap();
    let a = member(store.as_ref(), ws.id, "a").await;
    let b = member(store.as_ref(), ws.id, "b").await;
    let c = member(store.as_ref(), ws.id, "c").await;
    let token = mint_token(store.as_ref(), ws.id, a).await;

    let open = client
        .post(format!("{base}/workspaces/{}/group-dms", ws.id))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "member_ids": [b.0, c.0] }))
        .send()
        .await
        .unwrap();
    assert_eq!(open.status(), StatusCode::CREATED);
    let group: serde_json::Value = open.json().await.unwrap();
    let ids: Vec<&str> = group["member_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(
        ids.contains(&a.0.to_string().as_str()),
        "the opener was added"
    );
    let gid = group["id"].as_str().unwrap();

    let msg = client
        .post(format!("{base}/group-dms/{gid}/messages"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "body": "the opener can post" }))
        .send()
        .await
        .unwrap();
    assert_eq!(msg.status(), StatusCode::CREATED);

    // Listing the opener explicitly does not add them twice.
    let listed = client
        .post(format!("{base}/workspaces/{}/group-dms", ws.id))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "member_ids": [a.0, b.0, c.0] }))
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::CREATED);
    let group: serde_json::Value = listed.json().await.unwrap();
    assert_eq!(group["member_ids"].as_array().unwrap().len(), 3);

    // A member of another workspace cannot open a group DM in this one.
    let other_ws = store
        .create_workspace(NewWorkspace {
            name: "elsewhere".into(),
        })
        .await
        .unwrap();
    let outsider = member(store.as_ref(), other_ws.id, "outsider").await;
    let outsider_token = mint_token(store.as_ref(), other_ws.id, outsider).await;
    let refused = client
        .post(format!("{base}/workspaces/{}/group-dms", ws.id))
        .header("Authorization", format!("Bearer {outsider_token}"))
        .json(&json!({ "member_ids": [a.0, b.0, c.0] }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    // Nor can it fold this workspace's members into a group in its own.
    let mixed = client
        .post(format!("{base}/workspaces/{}/group-dms", other_ws.id))
        .header("Authorization", format!("Bearer {outsider_token}"))
        .json(&json!({ "member_ids": [b.0, c.0] }))
        .send()
        .await
        .unwrap();
    assert_eq!(mixed.status(), StatusCode::NOT_FOUND);

    server.abort();
}
