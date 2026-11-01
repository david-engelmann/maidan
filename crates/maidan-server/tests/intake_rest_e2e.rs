//! The intake queue over HTTP, auth ENABLED: an item is proposed, triaged
//! once (accepted into a new or existing thread, or rejected with a reason),
//! announced to the workspace's live subscribers, and audited on triage.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
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

struct Seeded {
    ws: WorkspaceId,
    me: MemberId,
    channel: ChannelId,
    token: String,
}

async fn seed(store: &Arc<dyn Store>, name: &str, kind: MemberKind) -> Seeded {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let me = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "me".into(),
            display_name: None,
            kind,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let caps = vec![
        capability::WORKSPACE_READ.into(),
        capability::WORKSPACE_WRITE.into(),
        capability::EVENT_SUBSCRIBE.into(),
    ];
    let token = mint(store.as_ref(), ws.id, me.id, caps).await;
    Seeded {
        ws: ws.id,
        me: me.id,
        channel: channel.id,
        token,
    }
}

async fn propose(http: &reqwest::Client, base: &str, s: &Seeded, title: &str) -> Value {
    let res = http
        .post(format!("{base}/workspaces/{}/intake", s.ws.0))
        .bearer_auth(&s.token)
        .json(&json!({ "title": title, "body": "Seen in the nightly run." }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    res.json().await.unwrap()
}

async fn triage(
    http: &reqwest::Client,
    base: &str,
    s: &Seeded,
    id: &str,
    body: Value,
) -> reqwest::Response {
    http.post(format!("{base}/workspaces/{}/intake/{id}/triage", s.ws.0))
        .bearer_auth(&s.token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// Proposing, accepting into a new thread, linking to an existing thread and
/// rejecting all reach the workspace's live subscribers; each triage is
/// audited; the list filters by state and pages newest first.
#[tokio::test]
async fn intake_is_proposed_triaged_announced_and_audited() {
    use futures::SinkExt;
    use std::time::Duration;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{client::IntoClientRequest, Message},
    };

    let (addr, http, store) = spawn().await;
    let base = format!("http://{addr}");
    let s = seed(&store, "home", MemberKind::Agent).await;

    let url = format!("ws://{addr}/ws/subscribe");
    let (mut sub, _) = connect_async(url.into_client_request().unwrap())
        .await
        .unwrap();
    sub.send(Message::Text(
        json!({ "token": s.token, "filter": {} }).to_string(),
    ))
    .await
    .unwrap();
    drain_ws(&mut sub, Duration::from_millis(300)).await;

    let first = propose(&http, &base, &s, "Add a retry budget").await;
    assert_eq!(first["state"], "proposed");
    assert_eq!(
        first["source"], "agent",
        "an agent's proposal defaults to agent"
    );
    assert_eq!(first["proposed_by"], s.me.0.to_string());
    let second = propose(&http, &base, &s, "Same retry budget").await;
    let third = propose(&http, &base, &s, "Rewrite everything").await;
    let id = |v: &Value| v["id"].as_str().unwrap().to_owned();

    let accepted = triage(
        &http,
        &base,
        &s,
        &id(&first),
        json!({ "decision": "accept", "channel_id": s.channel.0 }),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted: Value = accepted.json().await.unwrap();
    assert_eq!(accepted["state"], "accepted");
    assert_eq!(accepted["triaged_by"], s.me.0.to_string());
    let thread_id = accepted["thread_id"].as_str().unwrap().to_owned();
    let thread = store
        .get_thread(maidan_types::ThreadId(thread_id.parse().unwrap()))
        .await
        .unwrap();
    assert_eq!(thread.title.as_deref(), Some("Add a retry budget"));

    let linked = triage(
        &http,
        &base,
        &s,
        &id(&second),
        json!({ "decision": "accept", "thread_id": thread_id }),
    )
    .await;
    assert_eq!(linked.status(), StatusCode::OK);
    let linked: Value = linked.json().await.unwrap();
    assert_eq!(linked["thread_id"], thread_id.as_str());

    let rejected = triage(
        &http,
        &base,
        &s,
        &id(&third),
        json!({ "decision": "reject", "reason": "duplicate" }),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::OK);
    let rejected: Value = rejected.json().await.unwrap();
    assert_eq!(rejected["state"], "rejected");
    assert_eq!(rejected["reject_reason"], "duplicate");

    let frames = drain_ws(&mut sub, Duration::from_millis(800)).await;
    let count = |kind: &str| {
        frames
            .iter()
            .filter(|f| f["kind"].as_str() == Some(kind))
            .count()
    };
    assert_eq!(count("intake_proposed"), 3, "{frames:?}");
    assert_eq!(count("intake_accepted"), 2, "{frames:?}");
    assert_eq!(count("intake_rejected"), 1, "{frames:?}");

    let audited = store.list_audit_for_workspace(s.ws, 50).await.unwrap();
    let accepts = audited
        .iter()
        .filter(|a| a.action == "intake.accept")
        .count();
    let rejects = audited
        .iter()
        .filter(|a| a.action == "intake.reject")
        .count();
    assert_eq!((accepts, rejects), (2, 1));

    let list = |query: &'static str| {
        let http = http.clone();
        let url = format!("{base}/workspaces/{}/intake{query}", s.ws.0);
        let token = s.token.clone();
        async move {
            let res = http.get(url).bearer_auth(token).send().await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            res.json::<Vec<Value>>().await.unwrap()
        }
    };
    let all = list("").await;
    assert_eq!(all.len(), 3);
    assert_eq!(all[0]["id"], third["id"], "newest first");
    assert_eq!(list("?state=accepted").await.len(), 2);
    assert_eq!(list("?state=proposed").await.len(), 0);
    let page = list("?limit=2").await;
    assert_eq!(page.len(), 2);
    let cursor = page[1]["id"].as_str().unwrap().to_owned();
    let rest = http
        .get(format!(
            "{base}/workspaces/{}/intake?limit=2&cursor={cursor}",
            s.ws.0
        ))
        .bearer_auth(&s.token)
        .send()
        .await
        .unwrap()
        .json::<Vec<Value>>()
        .await
        .unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0]["id"], first["id"]);

    let one = http
        .get(format!(
            "{base}/workspaces/{}/intake/{}",
            s.ws.0,
            id(&third)
        ))
        .bearer_auth(&s.token)
        .send()
        .await
        .unwrap();
    assert_eq!(one.status(), StatusCode::OK);
}

/// A rejection needs a reason, an accept needs exactly one target, a
/// triaged item cannot be triaged again, and another workspace's item or
/// thread is not found.
#[tokio::test]
async fn triage_is_validated_once_only_and_workspace_scoped() {
    let (addr, http, store) = spawn().await;
    let base = format!("http://{addr}");
    let home = seed(&store, "home", MemberKind::Human).await;
    let away = seed(&store, "away", MemberKind::Human).await;

    let item = propose(&http, &base, &home, "Ship the thing").await;
    assert_eq!(item["source"], "human");
    let id = item["id"].as_str().unwrap().to_owned();

    for body in [
        json!({ "decision": "reject" }),
        json!({ "decision": "reject", "reason": "   " }),
        json!({ "decision": "accept" }),
        json!({ "decision": "accept", "channel_id": home.channel.0, "thread_id": home.channel.0 }),
    ] {
        let res = triage(&http, &base, &home, &id, body.clone()).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{body}");
    }

    let blank = http
        .post(format!("{base}/workspaces/{}/intake", home.ws.0))
        .bearer_auth(&home.token)
        .json(&json!({ "title": "  " }))
        .send()
        .await
        .unwrap();
    assert_eq!(blank.status(), StatusCode::BAD_REQUEST);

    // Another workspace's channel is not a target.
    let foreign = triage(
        &http,
        &base,
        &home,
        &id,
        json!({ "decision": "accept", "channel_id": away.channel.0 }),
    )
    .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);

    // Another workspace cannot see or triage the item.
    let peek = http
        .get(format!("{base}/workspaces/{}/intake/{id}", away.ws.0))
        .bearer_auth(&away.token)
        .send()
        .await
        .unwrap();
    assert_eq!(peek.status(), StatusCode::NOT_FOUND);
    let poke = triage(
        &http,
        &base,
        &away,
        &id,
        json!({ "decision": "reject", "reason": "mine now" }),
    )
    .await;
    assert_eq!(poke.status(), StatusCode::NOT_FOUND);
    let cross = http
        .get(format!("{base}/workspaces/{}/intake", home.ws.0))
        .bearer_auth(&away.token)
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), StatusCode::FORBIDDEN);

    let ok = triage(
        &http,
        &base,
        &home,
        &id,
        json!({ "decision": "reject", "reason": "out of scope" }),
    )
    .await;
    assert_eq!(ok.status(), StatusCode::OK);
    let again = triage(
        &http,
        &base,
        &home,
        &id,
        json!({ "decision": "reject", "reason": "still out of scope" }),
    )
    .await;
    assert_eq!(again.status(), StatusCode::CONFLICT);

    let audited = store.list_audit_for_workspace(home.ws, 50).await.unwrap();
    assert_eq!(
        audited
            .iter()
            .filter(|a| a.action.starts_with("intake."))
            .count(),
        1
    );
    assert!(store
        .list_audit_for_workspace(away.ws, 50)
        .await
        .unwrap()
        .iter()
        .all(|a| !a.action.starts_with("intake.")));
}
