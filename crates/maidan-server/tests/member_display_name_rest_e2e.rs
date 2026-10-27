//! A member renaming themself over HTTP. Auth ENABLED: the change is
//! announced to live subscribers of the member's own workspace and audited,
//! in one transaction with the change.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
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
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg
            && let Ok(frame) = serde_json::from_str(&text)
        {
            frames.push(frame);
        }
    }
    frames
}

/// Setting and then clearing a display name reaches the live subscribers of
/// the member's workspace, and is audited there, and no one else's.
#[tokio::test]
async fn a_display_name_change_is_announced_to_its_own_workspace_only() {
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
            let me = store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: "me".into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap();
            let caps = vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::EVENT_SUBSCRIBE.into(),
            ];
            let token = mint(store.as_ref(), ws.id, me.id, caps).await;
            (ws.id, me.id, token)
        }
    };
    let (home_ws, me, mine) = seed("home").await;
    let (away_ws, _, stranger) = seed("away").await;

    let mut subscribers = Vec::new();
    for token in [&mine, &stranger] {
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

    for name in ["Ada Lovelace", ""] {
        let renamed = client
            .patch(format!("{base}/members/{}", me.0))
            .header("Authorization", format!("Bearer {mine}"))
            .json(&serde_json::json!({ "display_name": name }))
            .send()
            .await
            .unwrap();
        assert_eq!(renamed.status(), StatusCode::OK);
    }

    let kind_of = |frame: &Value| frame["kind"].as_str().map(str::to_owned);
    let home = drain_ws(&mut subscribers[0], Duration::from_millis(800)).await;
    let changes: Vec<&Value> = home
        .iter()
        .filter(|f| kind_of(f).as_deref() == Some("member_display_name_changed"))
        .collect();
    assert_eq!(changes.len(), 2, "expected a set and a clear: {home:?}");
    assert_eq!(changes[0]["member_id"], me.0.to_string());
    assert_eq!(changes[0]["display_name"], "Ada Lovelace");
    assert_eq!(changes[0]["attribution"]["actor_id"], me.0.to_string());
    assert!(changes[1].get("display_name").is_none_or(Value::is_null));

    let away = drain_ws(&mut subscribers[1], Duration::from_millis(300)).await;
    assert!(
        away.iter()
            .all(|f| kind_of(f).as_deref() != Some("member_display_name_changed")),
        "another workspace's subscriber saw the rename: {away:?}"
    );

    let audited = store.list_audit_for_workspace(home_ws, 50).await.unwrap();
    let renames = audited
        .iter()
        .filter(|a| a.action == "member.display_name.set" && a.target_id == Some(me.0))
        .count();
    assert_eq!(renames, 2);
    let elsewhere = store.list_audit_for_workspace(away_ws, 50).await.unwrap();
    assert!(elsewhere
        .iter()
        .all(|a| a.action != "member.display_name.set"));
}

/// A token can rename only its own member: another member of the same
/// workspace is forbidden, and a member of another workspace is not found.
/// Neither attempt leaves a rename event or audit row anywhere, and the
/// other workspace's export carries none of this workspace's renames.
#[tokio::test]
async fn renaming_someone_else_is_refused_and_no_export_carries_another_workspaces_rename() {
    let (addr, http, store) = spawn().await;
    let base = format!("http://{addr}");
    let seed = |name: &'static str, handle: &'static str| {
        let store = store.clone();
        async move {
            let ws = store
                .create_workspace(NewWorkspace { name: name.into() })
                .await
                .unwrap();
            let member = store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                })
                .await
                .unwrap();
            (ws.id, member.id)
        }
    };
    let (home, me) = seed("home", "me").await;
    let neighbour = store
        .create_member(NewMember {
            workspace_id: home,
            handle: "neighbour".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id;
    let (away, stranger) = seed("away", "stranger").await;
    let token = mint(
        store.as_ref(),
        home,
        me,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
    )
    .await;

    let rename = |target: MemberId| {
        http.patch(format!("{base}/members/{}", target.0))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "display_name": "Mallory" }))
            .send()
    };
    assert_eq!(
        rename(neighbour).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        rename(stranger).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    for id in [neighbour, stranger] {
        assert_eq!(store.get_member(id).await.unwrap().display_name, None);
    }

    assert_eq!(rename(me).await.unwrap().status(), StatusCode::OK);

    let kind = "member_display_name_changed";
    let count = |ws: WorkspaceId| {
        let store = store.clone();
        async move {
            let events = store.list_events_after(ws, 0, 500).await.unwrap();
            let audits = store.list_audit_for_workspace(ws, 500).await.unwrap();
            (
                events
                    .iter()
                    .filter(|e| e.kind == maidan_types::EventKind::MemberDisplayNameChanged)
                    .count(),
                audits
                    .iter()
                    .filter(|a| a.action == "member.display_name.set")
                    .count(),
            )
        }
    };
    assert_eq!(count(home).await, (1, 1));
    assert_eq!(count(away).await, (0, 0));

    let away_export =
        serde_json::to_string(&maidan_server::export::build(&store, away).await.unwrap()).unwrap();
    assert!(
        !away_export.contains(kind),
        "another workspace's rename leaked into an export"
    );
    assert!(!away_export.contains("Mallory"));
}
