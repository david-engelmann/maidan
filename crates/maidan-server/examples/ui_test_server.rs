//! Seed-and-serve harness for the Playwright `/ui` suite.
//!
//! This is **test support, not a shipped binary**: it stands up the real
//! `maidan-server` router on an in-memory SQLite store, seeds a deterministic
//! workspace / channel / thread / pending approval gate (plus a `build` channel
//! with one thread per board lane), mints a bearer token,
//! writes the fixtures to a JSON file, and then serves forever so a headless
//! browser can drive the actual `/ui`. Playwright's `webServer` starts it,
//! waits for `/ui/`, runs the specs, and kills it.
//!
//! Env: `UI_TEST_PORT` (default 8899), `UI_TEST_FIXTURES` (default
//! `ui-tests/.fixtures.json`). Run via `cargo run --example ui_test_server`.

use std::net::SocketAddr;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_fsm::ThreadAction;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewApprovalGate, NewChannel, NewMember, NewMessage, NewThread,
    NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("UI_TEST_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8899);
    let fixtures_path =
        std::env::var("UI_TEST_FIXTURES").unwrap_or_else(|_| "ui-tests/.fixtures.json".into());

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));

    // Deterministic seed the specs assert against.
    let ws = store
        .create_workspace(NewWorkspace {
            name: "UI Test Workspace".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "operator".into(),
            display_name: Some("Operator".into()),
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("Deploy v9".into()),
        })
        .await
        .expect("thread");
    // The agent that asked. The operator answers it, and no one accepts their
    // own request.
    let requester = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "deployer".into(),
            display_name: Some("Deployer".into()),
            kind: MemberKind::Agent,
        })
        .await
        .expect("requester");
    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: requester.id,
            prompt: "Deploy v9 to prod?".into(),
            schema: None,
        })
        .await
        .expect("gate");

    // A second channel laid out as a board: one thread in each lane, so the
    // board, the state badges and the name rendering have something real to
    // show. Kept out of "general" so the specs that pick the first thread
    // there are unaffected.
    let build = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "build".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("build channel");
    let board_thread = |title: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_thread(NewThread {
                    channel_id: build.id,
                    parent_thread_id: None,
                    title: Some(title.into()),
                })
                .await
                .expect("board thread")
        }
    };
    let open_thread = board_thread("Open: nobody holds this").await;
    let claimed_thread = board_thread("Claimed: the deployer holds this").await;
    store
        .claim_thread(claimed_thread.id, requester.id)
        .await
        .expect("claim");
    let review_thread = board_thread("Review: result waiting on a reviewer").await;
    store
        .claim_thread(review_thread.id, requester.id)
        .await
        .expect("claim for review");
    store
        .post_message(NewMessage {
            thread_id: review_thread.id,
            author_id: requester.id,
            body: "Done; result attached.".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("message");
    store
        .set_thread_result(
            review_thread.id,
            requester.id,
            &serde_json::json!({ "status": "fixed" }),
        )
        .await
        .expect("result");
    store
        .transition_thread(review_thread.id, requester.id, ThreadAction::StartReview)
        .await
        .expect("start review");
    let done_thread = board_thread("Done: closed").await;
    store
        .transition_thread(done_thread.id, member.id, ThreadAction::StartReview)
        .await
        .expect("review before close");
    store
        .transition_thread(done_thread.id, member.id, ThreadAction::Close)
        .await
        .expect("close");

    // A review desk: three tasks the deployer handed to review, each naming the
    // operator as its one required reviewer, for the "Needs you" inbox. Its
    // own channel, so approving and sending back here leaves the board alone.
    let desk = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "desk".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("desk channel");
    let mut desk_threads = Vec::new();
    for (title, result) in [
        (
            "Approve me: the flaky login test is fixed",
            serde_json::json!({ "runs": 500, "failures": 0 }),
        ),
        (
            "Send me back: the rate limit",
            serde_json::json!({ "limit_per_min": 60 }),
        ),
        (
            "Still waiting: the upload path",
            serde_json::json!({ "p95_ms": 180 }),
        ),
    ] {
        let t = store
            .create_thread(NewThread {
                channel_id: desk.id,
                parent_thread_id: None,
                title: Some(title.into()),
            })
            .await
            .expect("desk thread");
        store.claim_thread(t.id, requester.id).await.expect("claim");
        store
            .set_thread_result(t.id, requester.id, &result)
            .await
            .expect("desk result");
        store
            .transition_thread(t.id, requester.id, ThreadAction::StartReview)
            .await
            .expect("desk review");
        store
            .set_review_requirement(t.id, 1)
            .await
            .expect("requirement");
        store.add_reviewer(t.id, member.id).await.expect("reviewer");
        desk_threads.push(t);
    }

    // A floor for the team strip and card motion: the deployer holds one
    // task, and one sits open for a spec to claim and watch glide into
    // Working. Its own channel, so moving a card here leaves the others alone.
    let floor = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "floor".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("floor channel");
    let mut floor_threads = Vec::new();
    for title in ["Held: the deployer is on this", "Glide me: claim this one"] {
        floor_threads.push(
            store
                .create_thread(NewThread {
                    channel_id: floor.id,
                    parent_thread_id: None,
                    title: Some(title.into()),
                })
                .await
                .expect("floor thread"),
        );
    }
    store
        .claim_thread(floor_threads[0].id, requester.id)
        .await
        .expect("floor claim");

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("ui-test".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
                capability::ARTIFACT_UPLOAD.into(),
            ],
            expires_at: None,
        })
        .await
        .expect("token");

    // The requesting agent's own token: a spec that opens a gate for the
    // operator to answer must ask as someone other than the operator.
    let requester_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: requester.id,
            app_installation_id: None,
            token_hash: hash_secret(requester_secret.as_str()),
            label: Some("ui-test-requester".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .expect("requester token");

    // The operator again, with `event:subscribe`: what the Live bar needs.
    let live_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(live_secret.as_str()),
            label: Some("ui-test-live".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::EVENT_SUBSCRIBE.into(),
            ],
            expires_at: None,
        })
        .await
        .expect("live token");

    // The operator as a reviewer: approving and closing are thread
    // transitions.
    let review_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(review_secret.as_str()),
            label: Some("ui-test-review".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .expect("review token");

    let art_dir = std::env::temp_dir().join(format!("maidan-ui-test-{}", std::process::id()));
    std::fs::create_dir_all(&art_dir).expect("art dir");
    let artifacts = Arc::new(LocalFsStore::new(&art_dir));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED — the specs drive the real bearer path
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    // The approval-gate `request_state` HMAC + subscribe-resume are secret-keyed.
    state.subscribe_resume_secret = Some(Arc::from(&b"ui-test-subscribe-resume-secret-32b"[..]));

    let fixtures = serde_json::json!({
        "base_url": format!("http://127.0.0.1:{port}"),
        "token": secret.as_str(),
        "requester_token": requester_secret.as_str(),
        "live_token": live_secret.as_str(),
        "workspace_id": ws.id.0.to_string(),
        "member_id": member.id.0.to_string(),
        "channel_id": channel.id.0.to_string(),
        "thread_id": thread.id.0.to_string(),
        "gate_id": gate.id.0.to_string(),
        "requester_id": requester.id.0.to_string(),
        "board_channel_id": build.id.0.to_string(),
        "board_open_thread_id": open_thread.id.0.to_string(),
        "board_claimed_thread_id": claimed_thread.id.0.to_string(),
        "board_review_thread_id": review_thread.id.0.to_string(),
        "board_done_thread_id": done_thread.id.0.to_string(),
        "review_token": review_secret.as_str(),
        "desk_channel_id": desk.id.0.to_string(),
        "desk_approve_thread_id": desk_threads[0].id.0.to_string(),
        "desk_send_back_thread_id": desk_threads[1].id.0.to_string(),
        "desk_waiting_thread_id": desk_threads[2].id.0.to_string(),
        "floor_channel_id": floor.id.0.to_string(),
        "floor_held_thread_id": floor_threads[0].id.0.to_string(),
        "floor_glide_thread_id": floor_threads[1].id.0.to_string(),
    });
    std::fs::write(
        &fixtures_path,
        serde_json::to_string_pretty(&fixtures).expect("fixtures json"),
    )
    .expect("write fixtures");

    let app = router(state);
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind (is UI_TEST_PORT free?)");
    eprintln!("ui_test_server: seeded + listening on http://{addr} (fixtures: {fixtures_path})");
    axum::serve(listener, app).await.expect("serve");
}
