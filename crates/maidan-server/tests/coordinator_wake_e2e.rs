//! scripts/coordinator-wake.sh against a live server with the notification
//! router running: a coordinator that follows a channel and an agent sleeps
//! through a notification that needs nothing (an assignment), and wakes with a
//! one-line digest when the agent asks a question (`needs_input`). This is the
//! whole path: the event, the router's row, `wait_for_notification` on
//! `/mcp/coordinator`, the script's filter and its digest.

use std::{
    path::PathBuf,
    process::Stdio,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{notification_router::NotificationRouter, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    BusEnvelope, DeclaredStatus, MemberKind, NewApiToken, NewChannel, NewMember, NewThread,
    NewWorkspace, StoredEvent,
};

fn has(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

async fn publish(state: &AppState, stored: &StoredEvent) {
    state
        .bus
        .publish(BusEnvelope::from_stored(stored).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn the_wake_script_sleeps_through_other_kinds_and_wakes_on_a_question() {
    if !(has("jq") && has("curl")) {
        eprintln!("skipped: the wake script needs jq and curl");
        return;
    }
    let pool = maidan_search::sqlite_pool_options()
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
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let notifications = NotificationRouter::spawn(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let ws = store
        .create_workspace(NewWorkspace {
            name: "swarm".into(),
        })
        .await
        .unwrap()
        .id;
    let mk = |handle: &str| NewMember {
        workspace_id: ws,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Agent,
    };
    let coordinator = store.create_member(mk("coordinator")).await.unwrap();
    let agent = store.create_member(mk("agent")).await.unwrap();
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
            title: Some("add retries to the uploader".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .follow_channel(coordinator.id, channel.id)
        .await
        .unwrap();
    store.follow_member(coordinator.id, agent.id).await.unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: coordinator.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("coordinator".into()),
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();

    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/coordinator-wake.sh");
    let mut child = tokio::process::Command::new("bash")
        .arg(&script)
        .args(["--max-wait", "90"])
        .env("MAIDAN_URL", format!("http://{addr}"))
        .env("MAIDAN_TOKEN", secret.as_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("run coordinator-wake.sh");
    // Let the router attach and the script reach its first wait.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Assigning the thread notifies the agent's followers, the coordinator
    // among them, of a kind that needs nothing from it.
    let (_, assigned) = store
        .assign_thread_with_event(thread.id, agent.id, agent.id, None)
        .await
        .unwrap();
    publish(&state, &assigned).await;
    let mut slept = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if store
            .list_notifications(coordinator.id, true, 10)
            .await
            .unwrap()
            .iter()
            .any(|n| n.kind == maidan_types::EventKind::ThreadAssignmentChanged)
        {
            slept = true;
            break;
        }
    }
    assert!(slept, "the router never wrote the assignment notification");
    // Long enough for the wait to see the row and go back to sleep.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "the script woke for a kind outside the attention set"
    );

    let (_, asked) = store
        .declare_thread_status(
            thread.id,
            DeclaredStatus::NeedsInput,
            "which branch should this target?".into(),
            agent.id,
        )
        .await
        .unwrap();
    publish(&state, &asked).await;

    let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .expect("the script did not wake on the question")
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{stdout}");
    assert!(lines[0].contains("status_declared"), "{stdout}");
    assert!(
        lines[0].contains(&format!("thread={}", thread.id.0)),
        "{stdout}"
    );
    assert!(
        lines[0].contains("\"add retries to the uploader\""),
        "{stdout}"
    );
    assert!(
        lines[0].contains("needs_input: which branch should this target?"),
        "{stdout}"
    );
    assert_eq!(lines[1], "1 pending: 1 status_declared");
    assert!(!stdout.contains("thread_assignment_changed"), "{stdout}");
    assert!(
        !stdout.contains(secret.as_str()) && !stderr.contains(secret.as_str()),
        "the token was printed"
    );
    // Nothing was marked read: that is the coordinator's job.
    let unread: Vec<_> = store
        .list_notifications(coordinator.id, true, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.kind)
        .collect();
    for kind in [
        maidan_types::EventKind::ThreadAssignmentChanged,
        maidan_types::EventKind::StatusDeclared,
    ] {
        assert!(
            unread.contains(&kind),
            "{kind:?} was marked read: {unread:?}"
        );
    }

    notifications.shutdown().await;
    server.abort();
}
