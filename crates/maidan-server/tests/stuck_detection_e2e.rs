//! Stuck detection over REST: a thread whose agent holds a live lease but
//! has shown no progress for `MAIDAN_STALL_SECS` reads back `stalled` on both
//! board reads (the channel's thread page and the single-thread read), with
//! the `last_progress_at` it was judged from. A thread whose agent just
//! posted, or whose agent declared it is waiting, is not stalled. Auth is
//! enabled with real tokens, and a second workspace neither reads the first
//! one's stalled thread nor sees it among its own.

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
    ChannelId, DeclaredStatus, MemberId, MemberKind, NewApiToken, NewChannel, NewMember,
    NewMessage, NewThread, NewWorkspace, ThreadId, Workspace,
};
use reqwest::StatusCode;
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

/// Short enough to wait out in a test, long enough that a thread whose agent
/// posted just before the read is not stalled on a slow runner.
const STALL_SECS: u64 = 2;

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
        .claim_next_thread(channel, agent, Some(600))
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

fn card<'a>(page: &'a Value, thread: ThreadId) -> &'a Value {
    page.as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == thread.0.to_string())
        .expect("the thread is on its channel's page")
}

fn stalled(thread: &Value) -> bool {
    thread
        .get("stalled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[tokio::test]
async fn a_quiet_agent_reads_back_stalled_and_only_in_its_own_workspace() {
    // Read per request, so setting it before the first read is enough; this
    // binary has one test, so nothing else sees it.
    std::env::set_var("MAIDAN_STALL_SECS", STALL_SECS.to_string());
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
    let app = router(state);
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

    let quiet = worked_task(&store, ours, agent, "quiet").await;
    let talking = worked_task(&store, ours, agent, "talking").await;
    let waiting = worked_task(&store, ours, agent, "waiting").await;
    let their_quiet = worked_task(&store, theirs, other_agent, "their quiet").await;
    store
        .declare_thread_status(
            waiting,
            DeclaredStatus::NeedsInput,
            "Which schema version?".into(),
            agent,
        )
        .await
        .unwrap();

    // Before the threshold nothing is stalled, though every worked thread
    // already has a progress clock.
    let (status, page) = get_json(
        &client,
        &format!("{base}/channels/{}/threads", ours.0),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for thread in [quiet, talking, waiting] {
        assert!(!stalled(card(&page, thread)), "fresh work is not stalled");
        assert!(card(&page, thread)["last_progress_at"].is_string());
    }

    tokio::time::sleep(Duration::from_millis(STALL_SECS * 1000 + 500)).await;
    store
        .post_message(NewMessage {
            thread_id: talking,
            author_id: agent,
            body: "Halfway through.".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();

    let (status, page) = get_json(
        &client,
        &format!("{base}/channels/{}/threads", ours.0),
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        stalled(card(&page, quiet)),
        "a quiet agent on a live lease is stalled"
    );
    assert!(
        !stalled(card(&page, talking)),
        "an agent that just posted is not"
    );
    assert!(
        !stalled(card(&page, waiting)),
        "an agent that declared it is waiting is not stalled"
    );
    assert!(
        page.as_array()
            .unwrap()
            .iter()
            .all(|t| t["id"] != their_quiet.0.to_string()),
        "another workspace's thread is not on this page"
    );

    let (status, one) = get_json(&client, &format!("{base}/threads/{}", quiet.0), &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert!(stalled(&one), "the single-thread read agrees");
    assert_eq!(
        one["last_progress_at"],
        card(&page, quiet)["last_progress_at"]
    );

    // The other workspace sees its own stalled thread and nothing of ours.
    let (status, their_page) = get_json(
        &client,
        &format!("{base}/channels/{}/threads", theirs.0),
        &other_auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(their_page.as_array().unwrap().len(), 1);
    assert!(stalled(card(&their_page, their_quiet)));
    for url in [
        format!("{base}/threads/{}", quiet.0),
        format!("{base}/channels/{}/threads", ours.0),
    ] {
        let (status, body) = get_json(&client, &url, &other_auth).await;
        assert!(
            matches!(status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
            "a cross-workspace read of {url} must be refused, got {status}"
        );
        assert!(body.get("stalled").is_none() && body.get("last_progress_at").is_none());
    }

    server.abort();
}
