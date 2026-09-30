//! Member-freeze kill-switch over HTTP. Auth ENABLED (the `frozen_by` FK + real
//! `token:admin` checks): freeze drops the member's lease, list/get surface it,
//! unfreeze lifts it, and a non-admin token is denied.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
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
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    // The WebSocket subscribe ack carries a resume token signed with this.
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn freeze_member_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let op = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
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
        })
        .await
        .unwrap();
    store.assign_thread(thread.id, agent.id).await.unwrap();

    let admin = mint(
        store.as_ref(),
        ws.id,
        op.id,
        vec![capability::TOKEN_ADMIN.into()],
    )
    .await;
    let admin_h = format!("Bearer {admin}");

    // Freeze the agent → drops the lease (released 1).
    let resp = client
        .post(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", &admin_h)
        .json(&serde_json::json!({ "reason": "compromised" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let result: Value = resp.json().await.unwrap();
    assert_eq!(result["released"], 1);
    assert_eq!(result["freeze"]["member_id"], agent.id.0.to_string());
    // The claimed thread is back in the queue.
    assert_eq!(store.get_thread(thread.id).await.unwrap().assignee_id, None);

    // Get + list surface the freeze.
    let got = client
        .get(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), StatusCode::OK);
    let list: Value = client
        .get(format!("{base}/workspaces/{}/frozen-members", ws.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);

    // A non-admin token is denied (403).
    let plain = mint(
        store.as_ref(),
        ws.id,
        op.id,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;
    let denied = client
        .post(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", format!("Bearer {plain}"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    // Unfreeze → gone (404 on repeat get / unfreeze).
    let del = client
        .delete(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let gone = client
        .get(format!("{base}/members/{}/freeze", agent.id.0))
        .header("Authorization", &admin_h)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

/// Every text frame a WebSocket subscriber receives within `window`.
async fn drain_ws<S>(ws: &mut S, window: std::time::Duration) -> Vec<Value>
where
    S: futures::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    use futures::StreamExt;
    let mut frames = Vec::new();
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, ws.next()).await {
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
            if let Ok(frame) = serde_json::from_str(&text) {
                frames.push(frame);
            }
        }
    }
    frames
}

/// A freeze and its lift reach the live subscribers of the frozen member's
/// workspace, naming the operator who did it, and no one else's.
#[tokio::test]
async fn a_freeze_is_announced_to_its_own_workspace_only() {
    use futures::SinkExt;
    use std::time::Duration;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{client::IntoClientRequest, Message},
    };

    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let seed = |name: &'static str| {
        let store = store.clone();
        async move {
            let ws = store
                .create_workspace(NewWorkspace { name: name.into() })
                .await
                .unwrap();
            let member = |handle: &str, kind| NewMember {
                workspace_id: ws.id,
                handle: handle.into(),
                display_name: None,
                kind,
            };
            let op = store
                .create_member(member("op", MemberKind::Human))
                .await
                .unwrap();
            let agent = store
                .create_member(member("agent", MemberKind::Agent))
                .await
                .unwrap();
            let caps = vec![
                capability::TOKEN_ADMIN.into(),
                capability::EVENT_SUBSCRIBE.into(),
            ];
            let token = mint(store.as_ref(), ws.id, op.id, caps).await;
            (ws.id, op.id, agent.id, token)
        }
    };
    let (_, op, agent, admin) = seed("home").await;
    let (_, _, _, stranger) = seed("away").await;

    let mut subscribers = Vec::new();
    for token in [&admin, &stranger] {
        let url = format!("ws://{addr}/ws/subscribe");
        let (mut ws, _) = connect_async(url.into_client_request().unwrap())
            .await
            .unwrap();
        ws.send(Message::Text(
            serde_json::json!({ "token": token, "filter": {} }).to_string(),
        ))
        .await
        .unwrap();
        drain_ws(&mut ws, Duration::from_millis(300)).await;
        subscribers.push(ws);
    }

    let frozen = client
        .post(format!("{base}/members/{}/freeze", agent.0))
        .header("Authorization", format!("Bearer {admin}"))
        .json(&serde_json::json!({ "reason": "runaway spend" }))
        .send()
        .await
        .unwrap();
    assert_eq!(frozen.status(), StatusCode::OK);
    let lifted = client
        .delete(format!("{base}/members/{}/freeze", agent.0))
        .header("Authorization", format!("Bearer {admin}"))
        .send()
        .await
        .unwrap();
    assert_eq!(lifted.status(), StatusCode::NO_CONTENT);

    let kind_of = |frame: &Value| frame["kind"].as_str().map(str::to_owned);
    let home = drain_ws(&mut subscribers[0], Duration::from_millis(800)).await;
    let frozen = home
        .iter()
        .find(|f| kind_of(f).as_deref() == Some("member_frozen"))
        .unwrap_or_else(|| panic!("no member_frozen frame: {home:?}"));
    assert_eq!(frozen["member_id"], agent.0.to_string());
    assert_eq!(frozen["frozen_by"], op.0.to_string());
    assert_eq!(frozen["reason"], "runaway spend");
    assert_eq!(frozen["attribution"]["actor_id"], op.0.to_string());
    assert!(
        home.iter()
            .any(|f| kind_of(f).as_deref() == Some("member_unfrozen")),
        "no member_unfrozen frame: {home:?}"
    );

    let away = drain_ws(&mut subscribers[1], Duration::from_millis(300)).await;
    assert!(
        away.iter().all(|f| !matches!(
            kind_of(f).as_deref(),
            Some("member_frozen" | "member_unfrozen")
        )),
        "another workspace's subscriber saw the freeze: {away:?}"
    );
}
