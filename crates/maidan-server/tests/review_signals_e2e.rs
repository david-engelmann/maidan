//! A change request tells the worker, and every verdict is in the log, with
//! auth on and two workspaces side by side. Each workspace's worker claims a
//! task, hands it to review and lets go; each reviewer requests changes over
//! REST. Each workspace's log has its own `review_submitted` and no other; the
//! notification router gives each worker one `review_submitted` notification
//! for their own thread and nothing from the other workspace; the reviewer
//! hears nothing about their own verdict, and an approval tells nobody.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{notification_router, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, Event, EventKind, Member, MemberId, MemberKind, NewApiToken, NewChannel, NewMember,
    NewThread, NewWorkspace, ReviewDecision, ThreadId, WorkspaceId,
};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::THREAD_TRANSITION.into(),
                capability::WORKSPACE_READ.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    format!("Bearer {}", secret.as_str())
}

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>, AppState) {
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
    let state = AppState::new(
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
    let app = router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store, state)
}

async fn call(
    client: &reqwest::Client,
    method: Method,
    url: String,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = client.request(method, url).header("Authorization", token);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

struct Tenant {
    ws: WorkspaceId,
    thread: ThreadId,
    worker: Member,
    reviewer: Member,
    worker_h: String,
    reviewer_h: String,
}

async fn tenant(store: &Arc<dyn Store>, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let mut members = Vec::new();
    for handle in ["worker", "reviewer"] {
        members.push(
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap(),
        );
    }
    let reviewer = members.pop().unwrap();
    let worker = members.pop().unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .unwrap()
        .id;
    Tenant {
        ws,
        thread,
        worker_h: mint(store.as_ref(), ws, worker.id).await,
        reviewer_h: mint(store.as_ref(), ws, reviewer.id).await,
        worker,
        reviewer,
    }
}

/// The worker takes the task, hands it to review and lets go.
async fn hand_in(client: &reqwest::Client, base: &str, store: &Arc<dyn Store>, t: &Tenant) {
    let cid = store.get_thread(t.thread).await.unwrap().channel_id.0;
    let (s, claim) = call(
        client,
        Method::POST,
        format!("{base}/channels/{cid}/threads/claim-next"),
        &t.worker_h,
        Some(json!({"lease_secs": 60})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{claim}");
    let tid = t.thread.0;
    let (s, _) = call(
        client,
        Method::POST,
        format!("{base}/threads/{tid}"),
        &t.worker_h,
        Some(json!({"action": "start_review"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(
        client,
        Method::POST,
        format!("{base}/threads/{tid}/claim/release"),
        &t.worker_h,
        Some(json!({"claim_lease_id": claim["claim_lease_id"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

async fn review(client: &reqwest::Client, base: &str, t: &Tenant, decision: &str) {
    let (s, body) = call(
        client,
        Method::POST,
        format!("{base}/threads/{}/reviews", t.thread.0),
        &t.reviewer_h,
        Some(json!({"decision": decision, "note": "cover the empty input"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
}

/// Route every event of both workspaces, as each replica's router would.
async fn route_all(state: &AppState, store: &Arc<dyn Store>, tenants: [&Tenant; 2]) {
    for t in tenants {
        for stored in store.list_events_after(t.ws, 0, 500).await.unwrap() {
            let event = stored.opened_event().unwrap();
            notification_router::route_event(state, stored.id, &event)
                .await
                .unwrap();
        }
    }
}

async fn inbox(client: &reqwest::Client, base: &str, member: &Member, token: &str) -> Vec<Value> {
    let (s, body) = call(
        client,
        Method::GET,
        format!("{base}/members/{}/notifications", member.id.0),
        token,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    body.as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "review_submitted")
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_change_request_notifies_the_last_worker_and_stays_in_its_workspace() {
    let (addr, client, store, state) = spawn().await;
    let base = format!("http://{addr}");
    let a = tenant(&store, "a").await;
    let b = tenant(&store, "b").await;

    for t in [&a, &b] {
        hand_in(&client, &base, &store, t).await;
        review(&client, &base, t, "request_changes").await;
    }

    // Each workspace's log holds its own verdict, and only that one.
    for (t, other) in [(&a, &b), (&b, &a)] {
        let (s, events) = call(
            &client,
            Method::GET,
            format!("{base}/workspaces/{}/events?types=review_submitted", t.ws.0),
            &t.worker_h,
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{events}");
        let events = events.as_array().unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        let payload = &events[0]["payload"];
        assert_eq!(events[0]["$type"], "maidan.event.review_submitted/1");
        assert_eq!(payload["thread_id"], json!(t.thread.0));
        assert_eq!(payload["reviewer_id"], json!(t.reviewer.id.0));
        assert_eq!(payload["decision"], "request_changes");
        assert_eq!(payload["sent_back"], true);
        assert_eq!(payload["worker_id"], json!(t.worker.id.0));
        assert!(payload.get("note").is_none(), "the note stays in history");
        assert_ne!(payload["thread_id"], json!(other.thread.0));
    }

    route_all(&state, &store, [&a, &b]).await;

    for (t, other) in [(&a, &b), (&b, &a)] {
        let got = inbox(&client, &base, &t.worker, &t.worker_h).await;
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0]["thread_id"], json!(t.thread.0));
        assert_eq!(got[0]["actor_id"], json!(t.reviewer.id.0));
        assert_eq!(got[0]["workspace_id"], json!(t.ws.0));
        assert_ne!(got[0]["thread_id"], json!(other.thread.0));
        assert!(
            inbox(&client, &base, &t.reviewer, &t.reviewer_h)
                .await
                .is_empty(),
            "a verdict is not news to the reviewer who gave it"
        );
    }

    // Routing again (a replay, a second replica) and an approval add nothing.
    review(&client, &base, &a, "approve").await;
    route_all(&state, &store, [&a, &b]).await;
    assert_eq!(inbox(&client, &base, &a.worker, &a.worker_h).await.len(), 1);
    let history = store.list_review_history(a.thread).await.unwrap();
    assert_eq!(
        history.iter().map(|v| v.decision).collect::<Vec<_>>(),
        [ReviewDecision::RequestChanges, ReviewDecision::Approve]
    );
    let logged = store
        .list_events_after(a.ws, 0, 500)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == EventKind::ReviewSubmitted)
        .count();
    assert_eq!(logged, 2, "every verdict is logged");
}

/// Recipients the event names directly are checked the way followers are: a
/// member of another workspace, the reviewer, the delegate that submitted the
/// verdict, and a worker who can no longer read the thread are not told.
#[tokio::test]
async fn a_change_request_is_not_routed_to_a_member_who_should_not_hear_it() {
    let (_, _, store, state) = spawn().await;
    let a = tenant(&store, "a").await;
    let b = tenant(&store, "b").await;
    let channel_id = store.get_thread(a.thread).await.unwrap().channel_id;
    let change_request = |worker_id: MemberId, actor_id: Option<MemberId>, channel: ChannelId| {
        Event::ReviewSubmitted {
            occurred_at: Utc::now(),
            workspace_id: a.ws,
            channel_id: channel,
            thread_id: a.thread,
            reviewer_id: a.reviewer.id,
            actor_id,
            decision: ReviewDecision::RequestChanges,
            sent_back: true,
            worker_id: Some(worker_id),
        }
    };
    let routes = [
        (b.worker.id, None),
        (a.reviewer.id, None),
        (a.worker.id, Some(a.worker.id)),
    ];
    for (log_id, (worker, actor)) in routes.into_iter().enumerate() {
        notification_router::route_event(
            &state,
            log_id as i64 + 1,
            &change_request(worker, actor, channel_id),
        )
        .await
        .unwrap();
    }
    for member in [b.worker.id, a.reviewer.id, a.worker.id] {
        assert!(
            store
                .list_notifications(member, false, 10)
                .await
                .unwrap()
                .is_empty(),
            "{member:?} was told"
        );
    }

    // A private thread the worker is not a member of.
    let private = store
        .create_channel(NewChannel {
            workspace_id: a.ws,
            name: "private".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    let hidden = store
        .create_thread(NewThread {
            channel_id: private.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let mut event = change_request(a.worker.id, None, private.id);
    if let Event::ReviewSubmitted { thread_id, .. } = &mut event {
        *thread_id = hidden.id;
    }
    notification_router::route_event(&state, 10, &event)
        .await
        .unwrap();
    assert!(store
        .list_notifications(a.worker.id, false, 10)
        .await
        .unwrap()
        .is_empty());

    // The same change request, to the worker it is meant for, is delivered.
    notification_router::route_event(&state, 11, &change_request(a.worker.id, None, channel_id))
        .await
        .unwrap();
    let got = store
        .list_notifications(a.worker.id, false, 10)
        .await
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].kind, EventKind::ReviewSubmitted);
    assert_eq!(got[0].thread_id, Some(a.thread));
    assert_eq!(got[0].actor_id, Some(a.reviewer.id));
}
