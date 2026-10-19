//! The human-timeout state machine over REST: an agent that declared it
//! needs a person reads back `waiting`, then `overdue` past half of
//! `MAIDAN_HUMAN_TIMEOUT_SECS`, and at the full timeout the claim reaper's
//! tick returns its thread to the queue: no holder, no stale declaration, the
//! thread still open, and the owner is told. Auth is enabled with real
//! tokens; a second workspace's wait, not yet due, keeps its holder, and that
//! workspace reads neither our thread nor its wait.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    BusEnvelope, ChannelId, DeclaredStatus, EventKind, MemberId, MemberKind, NewApiToken,
    NewChannel, NewMember, NewThread, NewWorkspace, ThreadId, Workspace,
};
use reqwest::StatusCode;
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

/// Whole seconds, so `overdue` (half) lands at one second and the return at
/// two, both short enough to wait out.
const TIMEOUT_SECS: u64 = 2;

async fn workspace_with_agent(store: &Arc<dyn Store>, name: &str) -> (Workspace, MemberId, String) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
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
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: agent.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some(name.into()),
            capabilities: vec!["workspace:read".into(), "thread:transition".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws, agent.id, format!("Bearer {}", secret.as_str()))
}

async fn channel(store: &Arc<dyn Store>, ws: &Workspace) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap()
        .id
}

/// A task the agent claims on a lease and acknowledges, so it is being worked.
async fn worked_task(
    store: &Arc<dyn Store>,
    channel: ChannelId,
    agent: MemberId,
    title: &str,
) -> ThreadId {
    let thread = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let claimed = store
        .claim_next_thread(channel, agent, Some(600), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, thread.id);
    store
        .acknowledge_claim(thread.id, agent, claimed.claim_lease_id.unwrap())
        .await
        .unwrap();
    thread.id
}

async fn get_json(client: &reqwest::Client, url: &str, auth: &str) -> (StatusCode, Value) {
    let resp = client
        .get(url)
        .header("Authorization", auth)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

fn card(page: &Value, thread: ThreadId) -> &Value {
    page.as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == thread.0.to_string())
        .expect("the thread is on its channel's page")
}

fn wait_state(thread: &Value) -> Option<&str> {
    thread["human_wait"]["state"].as_str()
}

async fn ask(store: &Arc<dyn Store>, thread: ThreadId, agent: MemberId) {
    store
        .declare_thread_status(
            thread,
            DeclaredStatus::NeedsInput,
            "Which schema version?".into(),
            agent,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn an_unanswered_wait_on_a_person_returns_the_thread_and_stays_in_its_workspace() {
    // Read per request and per tick; each test runs in its own process under
    // nextest, so nothing else sees it.
    std::env::set_var("MAIDAN_HUMAN_TIMEOUT_SECS", TIMEOUT_SECS.to_string());
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

    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(64)),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let (ws, agent, auth) = workspace_with_agent(&store, "a").await;
    let (other_ws, other_agent, other_auth) = workspace_with_agent(&store, "b").await;
    let ours = channel(&store, &ws).await;
    let theirs = channel(&store, &other_ws).await;

    let asking = worked_task(&store, ours, agent, "asking").await;
    let busy = worked_task(&store, ours, agent, "busy").await;
    // The person accountable for the thread, who must hear that the agent's
    // ask went unanswered and the work is back in the queue.
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "lead".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id;
    store.set_thread_owner(asking, Some(owner)).await.unwrap();
    let their_asking = worked_task(&store, theirs, other_agent, "their asking").await;
    ask(&store, asking, agent).await;

    let (status, page) = get_json(
        &client,
        &format!("{base}/channels/{}/threads", ours.0),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wait_state(card(&page, asking)), Some("waiting"));
    assert!(card(&page, asking)["human_wait"]["returns_at"].is_string());
    assert!(
        card(&page, busy).get("human_wait").is_none(),
        "an agent not waiting on a person has no clock"
    );

    tokio::time::sleep(Duration::from_millis(1200)).await;
    let (status, one) = get_json(&client, &format!("{base}/threads/{}", asking.0), &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wait_state(&one), Some("overdue"), "past half the timeout");

    // Theirs starts waiting now, so it is not due when ours is.
    ask(&store, their_asking, other_agent).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let returned =
        maidan_server::human_wait::return_timed_out_once(&state, Duration::from_secs(TIMEOUT_SECS))
            .await;
    assert_eq!(returned, 1, "only our wait is due");

    let (status, one) = get_json(&client, &format!("{base}/threads/{}", asking.0), &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["state"], "open", "nothing is decided for the person");
    assert!(one["assignee_id"].is_null(), "back in the queue: {one}");
    assert!(
        one.get("status").is_none(),
        "the holder's declaration went with it"
    );
    assert!(one.get("human_wait").is_none());
    let (_, busy_read) = get_json(&client, &format!("{base}/threads/{}", busy.0), &auth).await;
    assert_eq!(busy_read["assignee_id"], agent.0.to_string());

    // The return is recorded as a WaitTimedOut carrying the ask, and routing
    // that event tells the owner.
    let timed_out = store
        .list_events_after(ws.id, 0, 500)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.kind == EventKind::WaitTimedOut && e.thread_id == Some(asking))
        .expect("the return appended a WaitTimedOut");
    assert_eq!(timed_out.payload["policy"], "return_to_queue");
    assert_eq!(timed_out.payload["reason"], "Which schema version?");
    let envelope = BusEnvelope::from_stored(&timed_out).unwrap();
    maidan_server::notification_router::route_event(&state, envelope.log_id, &envelope.event)
        .await
        .unwrap();
    let told = store.list_notifications(owner, false, 10).await.unwrap();
    assert!(
        told.iter()
            .any(|n| n.kind == EventKind::WaitTimedOut && n.thread_id == Some(asking)),
        "the owner hears the wait went unanswered: {told:?}"
    );

    // Theirs keeps its holder and its clock; it reads neither our thread nor
    // our wait.
    let (status, their_page) = get_json(
        &client,
        &format!("{base}/channels/{}/threads", theirs.0),
        &other_auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(their_page.as_array().unwrap().len(), 1);
    let their_card = card(&their_page, their_asking);
    assert_eq!(their_card["assignee_id"], other_agent.0.to_string());
    assert!(
        matches!(wait_state(their_card), Some("waiting" | "overdue")),
        "still on its clock: {their_card}"
    );
    for url in [
        format!("{base}/threads/{}", asking.0),
        format!("{base}/channels/{}/threads", ours.0),
    ] {
        let (status, body) = get_json(&client, &url, &other_auth).await;
        assert!(
            matches!(status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
            "a cross-workspace read of {url} must be refused, got {status}"
        );
        assert!(body.get("human_wait").is_none());
    }

    server.abort();
}
