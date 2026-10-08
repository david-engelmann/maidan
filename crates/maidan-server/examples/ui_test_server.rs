//! Seed-and-serve harness for the Playwright `/ui` suite.
//!
//! This is **test support, not a shipped binary**: it stands up the real
//! `maidan-server` router on an in-memory SQLite store, seeds a deterministic
//! workspace / channel / thread / pending approval gate (plus a `build` channel
//! with one thread per board lane, and a second workspace for isolation
//! checks), mints a bearer token,
//! writes the fixtures to a JSON file, and then serves forever so a headless
//! browser can drive the actual `/ui`. Playwright's `webServer` starts it,
//! waits for `/ui/`, runs the specs, and kills it.
//!
//! Env: `UI_TEST_PORT` (default 8899), `UI_TEST_FIXTURES` (default
//! `ui-tests/.fixtures.json`). Run via `cargo run --example ui_test_server`.

// The fixtures `json!` literal outgrew the default macro recursion limit once
// the member-picker and feedback fixtures both landed.
#![recursion_limit = "256"]

use std::net::SocketAddr;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_fsm::ThreadAction;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ArtifactKind, BlockedReason, MemberKind, NewApiToken, NewApprovalGate, NewArtifact, NewChannel,
    NewMaidanSession, NewMember, NewMessage, NewThread, NewWebhookSubscription, NewWorkspace,
    ReviewDecision,
};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;

/// Signs every browser session the harness serves: the token exchange's and
/// the seeded signed-in session's.
const UI_SESSION_SECRET: &[u8] = b"ui-test-session-secret-at-least-32-bytes";

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
            description: None,
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
                    description: None,
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
    // Approved before it closed, so the card is plain "done" and not "closed
    // without review".
    let handed = store
        .latest_review_packet(done_thread.id)
        .await
        .expect("packet")
        .expect("handed to review");
    store
        .submit_review(
            done_thread.id,
            requester.id,
            ReviewDecision::Approve,
            None,
            Some(&handed.evidence_root),
        )
        .await
        .expect("approve before close");
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
                description: None,
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
    // task, and two sit open for specs to claim and watch move into Working
    // (one with motion, one with reduced motion). Its own channel, so moving a card here leaves the others alone.
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
    for title in [
        "Held: the deployer is on this",
        "Glide me: claim this one",
        "Jump me: claim this one with reduced motion",
    ] {
        floor_threads.push(
            store
                .create_thread(NewThread {
                    channel_id: floor.id,
                    parent_thread_id: None,
                    title: Some(title.into()),
                    description: None,
                })
                .await
                .expect("floor thread"),
        );
    }
    store
        .claim_thread(floor_threads[0].id, requester.id)
        .await
        .expect("floor claim");

    // Triage: reviews that name no reviewer. The operator owns one, which has
    // a result and no review requirement, so it may close it without review.
    // Nobody owns the other, which has no result: it falls to the workspace's
    // admins, and the operator holds a token:admin token.
    let triage = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "triage".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("triage channel");
    let mut triage_threads = Vec::new();
    for (title, owned, result) in [
        (
            "Nobody named: the retry budget",
            true,
            Some(serde_json::json!({ "status": "done" })),
        ),
        ("Nobody named, nobody owns: the cache header", false, None),
    ] {
        let t = store
            .create_thread(NewThread {
                channel_id: triage.id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
            })
            .await
            .expect("triage thread");
        store.claim_thread(t.id, requester.id).await.expect("claim");
        if owned {
            store
                .set_thread_owner(t.id, Some(member.id))
                .await
                .expect("owner");
        }
        if let Some(result) = result {
            store
                .set_thread_result(t.id, requester.id, &result)
                .await
                .expect("triage result");
        }
        store
            .transition_thread(t.id, requester.id, ThreadAction::StartReview)
            .await
            .expect("triage review");
        triage_threads.push(t);
    }

    // A task the operator owns, for the Unblock row in Needs you. It starts
    // unblocked: the spec blocks it, so a retry starts from the same place.
    // Its own channel, so the blocked marker on its card moves no other spec.
    let hold = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "hold".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("hold channel");
    let hold_thread = store
        .create_thread(NewThread {
            channel_id: hold.id,
            parent_thread_id: None,
            title: Some("Held up: the signing key needs a person".into()),
            description: None,
        })
        .await
        .expect("hold thread");
    store
        .set_thread_owner(hold_thread.id, Some(member.id))
        .await
        .expect("hold owner");

    // A task the operator owns and the deployer agent holds, for the
    // Question row in Needs you. The spec asks the question itself (an answer
    // clears it), so a retry starts from the same place.
    let ask = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "ask".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ask channel");
    let ask_thread = store
        .create_thread(NewThread {
            channel_id: ask.id,
            parent_thread_id: None,
            title: Some("Asked: where the replica runs".into()),
            description: None,
        })
        .await
        .expect("ask thread");
    store
        .set_thread_owner(ask_thread.id, Some(member.id))
        .await
        .expect("ask owner");
    store
        .claim_thread(ask_thread.id, requester.id)
        .await
        .expect("ask claim");

    // A second workspace, so a spec can show that one workspace sees nothing
    // of another. Its member owns a task that is already blocked, which its
    // own Needs you lists and the first workspace's never does.
    let second_ws = store
        .create_workspace(NewWorkspace {
            name: "Second UI Test Workspace".into(),
        })
        .await
        .expect("second ws");
    let stranger = store
        .create_member(NewMember {
            workspace_id: second_ws.id,
            handle: "stranger".into(),
            display_name: Some("Stranger".into()),
            kind: MemberKind::Human,
        })
        .await
        .expect("stranger");
    let afar = store
        .create_channel(NewChannel {
            workspace_id: second_ws.id,
            name: "afar".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("second channel");
    let second_thread = store
        .create_thread(NewThread {
            channel_id: afar.id,
            parent_thread_id: None,
            title: Some("Afar: the second workspace's blocked task".into()),
            description: None,
        })
        .await
        .expect("second thread");
    store
        .set_thread_owner(second_thread.id, Some(stranger.id))
        .await
        .expect("second owner");
    store
        .set_thread_block(
            second_thread.id,
            BlockedReason::Human,
            stranger.id,
            Some("only the second workspace may see this".into()),
        )
        .await
        .expect("second block");
    let second_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: second_ws.id,
            member_id: stranger.id,
            app_installation_id: None,
            token_hash: hash_secret(second_secret.as_str()),
            label: Some("ui-test-second".into()),
            capabilities: capability::all(),
            expires_at: None,
        })
        .await
        .expect("second token");

    // An empty channel, for the onboarding state a channel shows before its
    // first task.
    let quiet = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "quiet".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("quiet channel");

    // Hostile data, for the injection audit: a member, a channel topic, a
    // task, a message, a result and a gate prompt that each carry markup and a
    // script URL. Every renderer must show them as text. Its own channel, so
    // no other spec sees it.
    const XSS: &str = "<img src=x onerror=\"window.__xss=1\"><script>window.__xss=1</script>";
    let mallory = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "mallory".into(),
            display_name: Some(format!("Mallory {XSS}")),
            kind: MemberKind::Agent,
        })
        .await
        .expect("mallory");
    let lab = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "lab".into(),
            topic: Some(format!("topic {XSS}")),
            private: false,
        })
        .await
        .expect("lab channel");
    let lab_thread = store
        .create_thread(NewThread {
            channel_id: lab.id,
            parent_thread_id: None,
            title: Some(format!("Title {XSS}")),
            description: None,
        })
        .await
        .expect("lab thread");
    store
        .claim_thread(lab_thread.id, mallory.id)
        .await
        .expect("mallory claims");
    store
        .post_message(NewMessage {
            thread_id: lab_thread.id,
            author_id: mallory.id,
            body: format!("Body {XSS} [link](javascript:window.__xss=1)"),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("lab message");
    store
        .set_thread_result(
            lab_thread.id,
            mallory.id,
            &serde_json::json!({
                format!("key {XSS}"): format!("value {XSS}"),
                "link": "javascript:window.__xss=1",
                "upper": "JAVASCRIPT:window.__xss=1",
                "spaced": " javascript:window.__xss=1",
                "data": "data:text/html,<script>window.__xss=1</script>",
                "nested": { "html": XSS },
            }),
        )
        .await
        .expect("lab result");
    store
        .transition_thread(lab_thread.id, mallory.id, ThreadAction::StartReview)
        .await
        .expect("lab review");
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(lab_thread.id),
            requested_by: mallory.id,
            prompt: format!("Prompt {XSS}"),
            schema: None,
        })
        .await
        .expect("lab gate");

    // A second person, so the member picker offers a human beside the agents.
    let rae = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "rae".into(),
            display_name: Some("Rae Reviewer".into()),
            kind: MemberKind::Human,
        })
        .await
        .expect("rae");

    // An evidence desk: tasks the deployer handed to review with evidence
    // linked, each naming the operator as a reviewer, for the approval card.
    // `view` is only looked at, `decide` is approved by a spec, `live` is
    // approved by Rae while the operator watches, and `empty` was handed
    // over with no result and no artifact. Its own channel, so deciding here
    // moves no other spec's row.
    let proof = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "proof".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("proof channel");
    let evidence = |name: &str, kind: ArtifactKind, body: &[u8]| {
        let store = store.clone();
        let new = NewArtifact {
            sha256: hex::encode(Sha256::digest(body)),
            size_bytes: i64::try_from(body.len()).expect("size"),
            mime_type: Some("application/octet-stream".into()),
            filename: Some(name.into()),
            kind,
            uploaded_by: Some(requester.id),
        };
        async move {
            store
                .upsert_artifact_with_event(new, Some(ws.id))
                .await
                .expect("evidence artifact")
                .0
                .sha256
        }
    };
    let screenshot = evidence("login-after.png", ArtifactKind::Screenshot, &[7u8; 2048]).await;
    let transcript = evidence(
        "test-run.log",
        ArtifactKind::Transcript,
        b"500 runs, 0 failures\n",
    )
    .await;
    let mut proof_threads = Vec::new();
    for (title, result, artifacts, reviewers) in [
        (
            "Evidence: the login fix, with a screenshot and a log",
            Some(serde_json::json!({ "runs": 500, "failures": 0 })),
            vec![screenshot.clone(), transcript.clone()],
            1,
        ),
        (
            "Evidence: approve this one",
            Some(serde_json::json!({ "status": "fixed" })),
            vec![transcript.clone()],
            1,
        ),
        (
            "Evidence: two reviewers",
            Some(serde_json::json!({ "status": "fixed" })),
            vec![screenshot.clone()],
            2,
        ),
        ("Evidence: nothing handed over", None, vec![], 1),
    ] {
        let t = store
            .create_thread(NewThread {
                channel_id: proof.id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
            })
            .await
            .expect("proof thread");
        store.claim_thread(t.id, requester.id).await.expect("claim");
        if let Some(result) = result {
            store
                .set_thread_result(t.id, requester.id, &result)
                .await
                .expect("proof result");
        }
        for sha in &artifacts {
            store
                .link_thread_artifact(t.id, sha, requester.id)
                .await
                .expect("link evidence");
        }
        store
            .transition_thread(t.id, requester.id, ThreadAction::StartReview)
            .await
            .expect("proof review");
        store
            .set_review_requirement(t.id, reviewers)
            .await
            .expect("requirement");
        store.add_reviewer(t.id, member.id).await.expect("reviewer");
        if reviewers > 1 {
            store
                .add_reviewer(t.id, rae.id)
                .await
                .expect("second reviewer");
        }
        proof_threads.push(t);
    }
    // `decided` needs the operator and Rae, and Rae approved it before the
    // page loaded: the row names her from the task's reviews. `unlinked` had
    // the screenshot dropped after the hand-off, and Rae linked the log, so
    // the row says who linked what and flags the hash the task lost.
    for title in [
        "Evidence: Rae approved before you looked",
        "Evidence: a screenshot dropped after the hand-off",
    ] {
        let t = store
            .create_thread(NewThread {
                channel_id: proof.id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
            })
            .await
            .expect("proof thread");
        store.claim_thread(t.id, requester.id).await.expect("claim");
        store
            .set_thread_result(
                t.id,
                requester.id,
                &serde_json::json!({ "status": "fixed" }),
            )
            .await
            .expect("proof result");
        store
            .link_thread_artifact(t.id, &screenshot, requester.id)
            .await
            .expect("link screenshot");
        store
            .link_thread_artifact(t.id, &transcript, rae.id)
            .await
            .expect("link log");
        store
            .transition_thread(t.id, requester.id, ThreadAction::StartReview)
            .await
            .expect("proof review");
        let decided = proof_threads.len() == 4;
        store
            .set_review_requirement(t.id, if decided { 2 } else { 1 })
            .await
            .expect("requirement");
        store.add_reviewer(t.id, member.id).await.expect("reviewer");
        if decided {
            store
                .add_reviewer(t.id, rae.id)
                .await
                .expect("second reviewer");
            let packet = store
                .latest_review_packet(t.id)
                .await
                .expect("packet read")
                .expect("packet");
            store
                .submit_review(
                    t.id,
                    rae.id,
                    ReviewDecision::Approve,
                    None,
                    Some(&packet.evidence_root),
                )
                .await
                .expect("rae approves");
        } else {
            store
                .unlink_thread_artifact(t.id, &screenshot)
                .await
                .expect("unlink screenshot");
        }
        proof_threads.push(t);
    }
    let rae_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: rae.id,
            app_installation_id: None,
            token_hash: hash_secret(rae_secret.as_str()),
            label: Some("ui-test-rae".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .expect("rae token");

    // A second workspace with its own people and token. The member picker and
    // the DM lists signed in here must show nothing of the first workspace,
    // and the first must show nothing of this one.
    let other_ws = store
        .create_workspace(NewWorkspace {
            name: "UI Test Neighbour".into(),
        })
        .await
        .expect("other ws");
    let visitor = store
        .create_member(NewMember {
            workspace_id: other_ws.id,
            handle: "visitor".into(),
            display_name: Some("Visitor".into()),
            kind: MemberKind::Human,
        })
        .await
        .expect("visitor");
    let outsider = store
        .create_member(NewMember {
            workspace_id: other_ws.id,
            handle: "outsider".into(),
            display_name: Some("Outsider".into()),
            kind: MemberKind::Agent,
        })
        .await
        .expect("outsider");
    let other_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: other_ws.id,
            member_id: visitor.id,
            app_installation_id: None,
            token_hash: hash_secret(other_secret.as_str()),
            label: Some("ui-test-neighbour".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .expect("neighbour token");

    // A tier desk: tasks handed to review whose evidence the server tiers
    // differently, each naming the operator as a reviewer. `attached` has a
    // link from Rae, who never worked it. `verified` carries a land-gate pass
    // from the Verifier, recorded before the hand-off. `self` is only the
    // deployer's own result and link, so its card warns. Its own channel, and
    // only looked at, so no other spec's row moves.
    let verifier = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "verifier".into(),
            display_name: Some("Verifier".into()),
            kind: MemberKind::Agent,
        })
        .await
        .expect("verifier");
    store
        .add_member_skill(verifier.id, maidan_types::LAND_GATE_SKILL)
        .await
        .expect("land-gate skill");
    let tiers = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "tiers".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("tiers channel");
    let mut tier_threads = Vec::new();
    for title in [
        "Tiers: a link from someone who never worked it",
        "Tiers: a land-gate pass at the hand-off",
        "Tiers: only the worker's own account",
    ] {
        let t = store
            .create_thread(NewThread {
                channel_id: tiers.id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
            })
            .await
            .expect("tier thread");
        store.claim_thread(t.id, requester.id).await.expect("claim");
        store
            .set_thread_result(
                t.id,
                requester.id,
                &serde_json::json!({ "status": "fixed" }),
            )
            .await
            .expect("tier result");
        tier_threads.push(t);
    }
    store
        .link_thread_artifact(tier_threads[0].id, &screenshot, rae.id)
        .await
        .expect("rae links");
    store
        .require_land_gate(tier_threads[1].id)
        .await
        .expect("arm the land gate");
    store
        .set_land_gate_pointer(
            tier_threads[1].id,
            verifier.id,
            maidan_types::LandGateStatus::Pass,
            Some(&transcript),
            None,
        )
        .await
        .expect("verifier passes");
    store
        .link_thread_artifact(tier_threads[2].id, &transcript, requester.id)
        .await
        .expect("deployer links");
    for t in &tier_threads {
        store
            .transition_thread(t.id, requester.id, ThreadAction::StartReview)
            .await
            .expect("tier review");
        store
            .set_review_requirement(t.id, 1)
            .await
            .expect("requirement");
        store.add_reviewer(t.id, member.id).await.expect("reviewer");
    }

    // The neighbour's own tiered hand-off: the Outsider's result and link,
    // with the Visitor as reviewer. Its card warns in its own console, and
    // the first workspace sees none of it.
    let yard = store
        .create_channel(NewChannel {
            workspace_id: other_ws.id,
            name: "yard".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("yard channel");
    let yard_thread = store
        .create_thread(NewThread {
            channel_id: yard.id,
            parent_thread_id: None,
            title: Some("Neighbour: the outsider's own account".into()),
            description: None,
        })
        .await
        .expect("yard thread");
    let yard_body = b"neighbour evidence\n";
    let yard_sha = store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: hex::encode(Sha256::digest(yard_body)),
                size_bytes: i64::try_from(yard_body.len()).expect("size"),
                mime_type: Some("text/plain".into()),
                filename: Some("yard.log".into()),
                kind: ArtifactKind::Transcript,
                uploaded_by: Some(outsider.id),
            },
            Some(other_ws.id),
        )
        .await
        .expect("yard artifact")
        .0
        .sha256;
    store
        .claim_thread(yard_thread.id, outsider.id)
        .await
        .expect("outsider claims");
    store
        .set_thread_result(
            yard_thread.id,
            outsider.id,
            &serde_json::json!({ "status": "done" }),
        )
        .await
        .expect("yard result");
    store
        .link_thread_artifact(yard_thread.id, &yard_sha, outsider.id)
        .await
        .expect("outsider links");
    store
        .transition_thread(yard_thread.id, outsider.id, ThreadAction::StartReview)
        .await
        .expect("yard review");
    store
        .set_review_requirement(yard_thread.id, 1)
        .await
        .expect("yard requirement");
    store
        .add_reviewer(yard_thread.id, visitor.id)
        .await
        .expect("yard reviewer");

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
                capability::THREAD_TRANSITION.into(),
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

    // An admin who mints throwaway tokens: a spec that rotates a token must
    // not rotate one another spec (or its own retry) depends on.
    let admin_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(admin_secret.as_str()),
            label: Some("ui-test-admin".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::TOKEN_ADMIN.into(),
            ],
            expires_at: None,
        })
        .await
        .expect("admin token");

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

    // An admin, so Connect an agent can create a member (bootstrap is on)
    // and mint the worker preset. The operator token above stays narrow.
    let admin_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(admin_secret.as_str()),
            label: Some("ui-test-admin".into()),
            capabilities: capability::all(),
            expires_at: None,
        })
        .await
        .expect("admin token");

    // One dead-lettered webhook so the Operator tab can replay it. The harness
    // does not run the webhook worker, and creating a subscription over HTTP
    // needs an encryption key this process does not have.
    let hook = store
        .create_webhook_subscription(NewWebhookSubscription {
            workspace_id: ws.id,
            url: "https://hooks.example.test/maidan".into(),
            label: Some("ui-test".into()),
            event_kinds: vec!["message_posted".into()],
            secret_ciphertext: "ui-test-ciphertext".into(),
        })
        .await
        .expect("webhook");
    let delivery_id = store
        .enqueue_webhook_delivery(hook.id, 1, "{}")
        .await
        .expect("delivery");
    store
        .quarantine_webhook_delivery(delivery_id)
        .await
        .expect("quarantine");

    // The operator signed in through the identity provider: the session row
    // an OIDC callback writes, with no token behind it, so a spec can drive the
    // console as a person rather than as a pasted token. Accepting a gate
    // needs this (or approval:grant); the harness has no identity provider,
    // so the row is written here and the spec sets its cookie.
    let signed_in = store
        .create_session(NewMaidanSession {
            workspace_id: ws.id,
            member_id: member.id,
            api_token_id: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(8),
        })
        .await
        .expect("signed-in session");
    let mut cookie_headers = axum::http::HeaderMap::new();
    maidan_server::session::set_session_cookie(
        &mut cookie_headers,
        signed_in.id,
        28_800,
        false,
        UI_SESSION_SECRET,
    )
    .expect("session cookie");
    let session_cookie = cookie_headers
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .and_then(|pair| pair.strip_prefix("maidan_session="))
        .expect("session cookie value")
        .to_string();

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
        true,  // bootstrap: the same member-create route the product calls
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    // The approval-gate `request_state` HMAC + subscribe-resume are secret-keyed.
    state.subscribe_resume_secret = Some(Arc::from(&b"ui-test-subscribe-resume-secret-32b"[..]));
    // A pasted token is exchanged for a browser session, as in production.
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: Arc::from(UI_SESSION_SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });

    let fixtures = serde_json::json!({
        "base_url": format!("http://127.0.0.1:{port}"),
        "token": secret.as_str(),
        "session_cookie": session_cookie,
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
        "admin_token": admin_secret.as_str(),
        "desk_channel_id": desk.id.0.to_string(),
        "desk_approve_thread_id": desk_threads[0].id.0.to_string(),
        "desk_send_back_thread_id": desk_threads[1].id.0.to_string(),
        "desk_waiting_thread_id": desk_threads[2].id.0.to_string(),
        "triage_channel_id": triage.id.0.to_string(),
        "triage_owned_thread_id": triage_threads[0].id.0.to_string(),
        "triage_ownerless_thread_id": triage_threads[1].id.0.to_string(),
        "hold_channel_id": hold.id.0.to_string(),
        "hold_thread_id": hold_thread.id.0.to_string(),
        "ask_channel_id": ask.id.0.to_string(),
        "ask_thread_id": ask_thread.id.0.to_string(),
        "second_workspace_id": second_ws.id.0.to_string(),
        "second_member_id": stranger.id.0.to_string(),
        "second_token": second_secret.as_str(),
        "second_thread_id": second_thread.id.0.to_string(),
        "floor_channel_id": floor.id.0.to_string(),
        "floor_held_thread_id": floor_threads[0].id.0.to_string(),
        "floor_glide_thread_id": floor_threads[1].id.0.to_string(),
        "floor_jump_thread_id": floor_threads[2].id.0.to_string(),
        "quiet_channel_id": quiet.id.0.to_string(),
        "lab_channel_id": lab.id.0.to_string(),
        "lab_thread_id": lab_thread.id.0.to_string(),
        "lab_member_id": mallory.id.0.to_string(),
        "rae_member_id": rae.id.0.to_string(),
        "rae_token": rae_secret.as_str(),
        "proof_channel_id": proof.id.0.to_string(),
        "proof_view_thread_id": proof_threads[0].id.0.to_string(),
        "proof_decide_thread_id": proof_threads[1].id.0.to_string(),
        "proof_live_thread_id": proof_threads[2].id.0.to_string(),
        "proof_empty_thread_id": proof_threads[3].id.0.to_string(),
        "proof_decided_thread_id": proof_threads[4].id.0.to_string(),
        "proof_unlinked_thread_id": proof_threads[5].id.0.to_string(),
        "proof_screenshot_sha": screenshot,
        "proof_transcript_sha": transcript,
        "other_workspace_id": other_ws.id.0.to_string(),
        "other_member_id": visitor.id.0.to_string(),
        "outsider_member_id": outsider.id.0.to_string(),
        "other_token": other_secret.as_str(),
        "tiers_channel_id": tiers.id.0.to_string(),
        "tiers_attached_thread_id": tier_threads[0].id.0.to_string(),
        "tiers_verified_thread_id": tier_threads[1].id.0.to_string(),
        "tiers_self_thread_id": tier_threads[2].id.0.to_string(),
        "verifier_member_id": verifier.id.0.to_string(),
        "other_tiers_thread_id": yard_thread.id.0.to_string(),
        "admin_token": admin_secret.as_str(),
        "delivery_id": delivery_id,
        "delivery_url": "https://hooks.example.test/maidan",
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
