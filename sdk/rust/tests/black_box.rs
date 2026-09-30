//! Black-box tests against the authenticated server from `scripts/sdk-test.sh`. Each
//! test skips (returns) when MAIDAN_URL is unset, matching the repo's Docker-skip
//! convention. These scenarios also exercise the server's REST + WS surface.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use maidan::{
    ArtifactKind, Channel, Client, ImportMode, MaidanError, MemberKind, Thread, ThreadState,
    PROBLEM_BASE,
};
use serde_json::{json, Value};

fn base() -> Option<String> {
    std::env::var("MAIDAN_URL").ok()
}

fn token() -> String {
    std::env::var("MAIDAN_TOKEN").unwrap_or_default()
}

fn workspace() -> String {
    std::env::var("MAIDAN_WORKSPACE").unwrap()
}

static SEED_ID: AtomicU64 = AtomicU64::new(1);

// Unique per run, so reruns against one server do not collide on names.
fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "{prefix}-{nanos}-{}",
        SEED_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn get_json(base: &str, path: &str) -> Value {
    ureq::get(&format!("{base}{path}"))
        .set("authorization", &format!("Bearer {}", token()))
        .call()
        .unwrap()
        .into_json()
        .unwrap()
}

// Create an isolated queue in the token's bootstrap workspace.
fn seed(c: &Client, base: &str) -> (String, String, Channel, Thread) {
    let wid = workspace();
    let me = get_json(base, "/me");
    let member_id = me["member_id"].as_str().unwrap().to_string();
    let channel = c
        .channels()
        .create(&wid, &unique("rust-sdk"), false)
        .unwrap();
    let thread = c.threads().create(&channel.id, "kickoff").unwrap();
    (wid, member_id, channel, thread)
}

/// Every member the server sent is declared on its model, all the way down.
/// A model keeps undeclared members in `extra` (forward compatibility), so
/// none anywhere is the proof that the models match what the server returns;
/// required members are enforced by serde itself.
macro_rules! assert_modeled {
    ($value:expr) => {{
        let value = $value;
        let unknown = value.unknown_members();
        assert!(
            unknown.is_empty(),
            "the server sent members the models do not declare: {unknown:?}"
        );
        value
    }};
}

fn assert_all_modeled<T>(rows: Vec<T>, unknown: impl Fn(&T) -> Vec<String>) -> Vec<T> {
    for row in &rows {
        let u = unknown(row);
        assert!(
            u.is_empty(),
            "the server sent members the models do not declare: {u:?}"
        );
    }
    rows
}

#[test]
fn hero_loop_post_list_context() {
    let Some(base) = base() else {
        eprintln!("skip: MAIDAN_URL unset");
        return;
    };
    let c = Client::new(&base, token());
    let (_wid, _member, _ch, thread) = seed(&c, &base);
    c.messages()
        .post(&thread.id, "hello from the rust sdk")
        .unwrap();
    let msgs = c.messages().list(&thread.id, &[]).unwrap();
    assert!(msgs.iter().any(|m| m.body == "hello from the rust sdk"));
    assert_eq!(
        c.threads().context(&thread.id, &[]).unwrap().thread.id,
        thread.id
    );
}

#[test]
fn get_result_unset_is_404() {
    // Exercise the result route and client error path before a result exists.
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, token());
    let (_wid, _m, _ch, thread) = seed(&c, &base);
    let err = c.threads().get_result(&thread.id).unwrap_err();
    assert!(matches!(err, MaidanError::NotFound(_)), "{err:?}");
    assert_eq!(err.status(), 404);
}

#[test]
fn errors_surface_status() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, token());
    let err = c
        .threads()
        .get("00000000-0000-0000-0000-000000000000")
        .unwrap_err();
    assert!(err.status() >= 400);
}

#[test]
fn claim_returns_the_thread_flattened_not_nested() {
    // The seeded thread is ready, so this claims it. A nested `thread` key
    // would land in `extra` and fail `assert_modeled`.
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, token());
    let (_wid, member, ch, thread) = seed(&c, &base);
    let claim = c
        .claim_next_thread(&ch.id, None)
        .unwrap()
        .expect("a freshly seeded ready thread should be claimable");
    assert_modeled!(&claim);
    assert_eq!(claim.id, thread.id);
    assert_eq!(claim.assignee_id.as_deref(), Some(member.as_str()));
    assert!(
        claim.claim_lease_id.is_some(),
        "the fencing token renew_claim needs"
    );
    assert!(!claim.pin.uri.is_empty() && !claim.pin.content_hash.is_empty());
}

#[test]
fn renew_claim_extends_the_lease_with_the_fencing_token() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, token());
    let (_wid, _member, ch, _t) = seed(&c, &base);
    let claim = c.claim_next_thread(&ch.id, Some(60)).unwrap().unwrap();
    let lease = claim.claim_lease_id.clone().unwrap();
    let renewed = c.renew_claim(&claim.id, &lease, 600).unwrap();
    assert!(
        renewed.assignment_expires_at > claim.assignment_expires_at,
        "lease not extended"
    );
}

#[test]
fn claim_next_returns_none_once_drained() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, token());
    let (_wid, _member, ch, _t) = seed(&c, &base);
    c.claim_next_thread(&ch.id, None).unwrap();
    assert!(c.claim_next_thread(&ch.id, None).unwrap().is_none());
}

#[test]
fn subscribe_delivers_a_message() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, token());
    let (wid, _member, _ch, thread) = seed(&c, &base);
    let (tx, rx) = mpsc::channel();
    let tid = thread.id.clone();
    let sub = c
        .subscribe(
            json!({ "workspace_id": wid, "kinds": ["message_posted"] }),
            move |e| {
                if e["thread_id"].as_str() == Some(tid.as_str()) {
                    let _ = tx.send(e);
                }
            },
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(200)); // let the subscription attach
    c.messages().post(&thread.id, "ws ping").unwrap();
    let e = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("did not receive the message_posted event");
    assert_eq!(e["kind"], "message_posted");
    sub.close();
}

#[test]
fn provisioning_seeds_a_member_and_mints_a_scoped_token() {
    // The first thing an integrator does after `maidan init`.
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, token());
    let wid = workspace();
    let handle = unique("provisioned");

    let member = c
        .members()
        .create(&wid, &handle, MemberKind::Agent, None)
        .unwrap();
    assert_eq!(member.handle, handle);
    assert_eq!(member.kind, MemberKind::Agent);
    assert!(c
        .members()
        .list(&wid)
        .unwrap()
        .iter()
        .any(|m| m.id == member.id));

    let minted = c
        .tokens()
        .mint(
            &wid,
            &member.id,
            &["workspace:read"],
            &maidan::MintOptions {
                label: Some("scoped worker"),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        !minted.secret.is_empty(),
        "the secret is returned once, in the mint response"
    );

    let tokens = c.tokens().list(&wid, &member.id).unwrap();
    assert!(
        tokens.iter().all(|t| !t.extra.contains_key("secret")),
        "listing must never return a secret"
    );
    assert!(tokens
        .iter()
        .any(|t| t.id == minted.id && t.label.as_deref() == Some("scoped worker")));
}

#[test]
fn threads_list_all_walks_every_page() {
    let Some(base) = base() else {
        eprintln!("skip: MAIDAN_URL unset");
        return;
    };
    let c = Client::new(&base, token());
    let (_wid, _member, channel, thread) = seed(&c, &base);
    let mut made = vec![thread.id.clone()];
    for i in 0..4 {
        made.push(
            c.threads()
                .create(&channel.id, &format!("t{i}"))
                .unwrap()
                .id,
        );
    }
    let mut seen: Vec<String> = c
        .threads()
        .list_all(&channel.id, 2)
        .map(|t| t.unwrap().id)
        .collect();
    made.sort();
    seen.sort();
    assert_eq!(seen, made);
}

#[test]
fn every_documented_operation_returns_its_declared_model() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, token());
    let (wid, member, channel, thread) = seed(&c, &base);

    assert_modeled!(c.workspaces().get(&wid).unwrap());
    assert_all_modeled(c.members().list(&wid).unwrap(), |m| m.unknown_members());
    assert_modeled!(&channel);
    assert_all_modeled(c.channels().list(&wid).unwrap(), |m| m.unknown_members());
    assert_modeled!(&thread);
    assert_modeled!(c.threads().get(&thread.id).unwrap());
    assert_all_modeled(c.threads().list(&channel.id, &[]).unwrap(), |m| {
        m.unknown_members()
    });

    let msg = assert_modeled!(c.messages().post(&thread.id, "typed").unwrap());
    assert_all_modeled(c.messages().list(&thread.id, &[]).unwrap(), |m| {
        m.unknown_members()
    });

    let art = assert_modeled!(c
        .artifacts()
        .upload(b"typed bytes", ArtifactKind::Attachment)
        .unwrap());
    assert_eq!(art.size_bytes, 11);
    assert_eq!(art.kind, ArtifactKind::Attachment);
    assert_modeled!(c.artifacts().meta(&art.sha256).unwrap());
    assert_eq!(c.artifacts().get(&art.sha256).unwrap(), b"typed bytes");

    let claim = assert_modeled!(c.claim_next_thread(&channel.id, Some(60)).unwrap().unwrap());
    let lease = claim.claim_lease_id.clone().unwrap();
    assert_modeled!(c.renew_claim(&claim.id, &lease, 120).unwrap());

    let result = assert_modeled!(c
        .threads()
        .set_result(&thread.id, json!({"ok": true}))
        .unwrap());
    assert_eq!(result.result, json!({"ok": true}));
    assert_eq!(result.produced_by, member);
    assert_modeled!(c.threads().get_result(&thread.id).unwrap());
    let reviewed = assert_modeled!(c.threads().transition(&thread.id, "start_review").unwrap());
    assert_eq!(reviewed.state, ThreadState::InReview);

    let ctx = assert_modeled!(c.threads().context(&thread.id, &[]).unwrap());
    assert!(
        !ctx.fsm.transitions.is_empty(),
        "the start_review transition is in the pack"
    );
    assert!(ctx.messages.iter().any(|m| m.id == msg.id));

    let events = assert_all_modeled(c.list_events(&wid, &[("limit", "50")]).unwrap(), |e| {
        e.unknown_members()
    });
    assert!(!events.is_empty());
    assert!(events
        .iter()
        .all(|e| e.event_type == maidan::event_type(&e.kind)));
    for e in c.list_events_all(&wid, &[("limit", "25")]) {
        assert_modeled!(e.unwrap());
    }

    let fresh = assert_modeled!(c
        .members()
        .create(&wid, &unique("typed"), MemberKind::Agent, None)
        .unwrap());
    assert_modeled!(c
        .tokens()
        .mint(&wid, &fresh.id, &["workspace:read"], &Default::default())
        .unwrap());
    assert_all_modeled(c.tokens().list(&wid, &fresh.id).unwrap(), |t| {
        t.unknown_members()
    });

    let bundle = get_json(&base, &format!("/workspaces/{wid}/export"));
    let imported = assert_modeled!(c
        .workspaces()
        .import(&bundle, Some(ImportMode::New))
        .unwrap());
    assert_eq!(imported.mode, ImportMode::New);
    assert_ne!(imported.workspace_id, wid, "mode=new remaps ids");
}

#[test]
fn the_servers_problem_types_arrive_as_their_variants() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, token());
    let (wid, _member, _channel, thread) = seed(&c, &base);
    let bundle = get_json(&base, &format!("/workspaces/{wid}/export"));
    let check = |err: MaidanError, status: u16, type_: &str| {
        let p = err.problem().expect("an HTTP problem");
        assert_eq!(p.status, status, "{err:?}");
        assert_eq!(
            p.problem_type.as_deref(),
            Some(format!("{PROBLEM_BASE}{type_}").as_str())
        );
        assert_eq!(
            p.raw.as_ref().and_then(|r| r["type"].as_str()),
            p.problem_type.as_deref(),
            "the raw problem is kept"
        );
        assert!(p.title.is_some() && p.detail.is_some());
        err
    };

    let e = check(
        c.threads()
            .get("00000000-0000-0000-0000-000000000000")
            .unwrap_err(),
        404,
        "not-found",
    );
    assert!(matches!(e, MaidanError::NotFound(_)));
    let e = check(
        Client::new(&base, "maid_not_a_token")
            .workspaces()
            .get(&wid)
            .unwrap_err(),
        401,
        "unauthorized",
    );
    assert!(matches!(e, MaidanError::Unauthorized(_)));
    let e = check(
        c.threads().transition(&thread.id, "fly").unwrap_err(),
        400,
        "bad-request",
    );
    assert!(matches!(e, MaidanError::BadRequest(_)));
    // Bootstrap creates only the first workspace; `maidan init` already made it.
    let e = check(
        c.workspaces().create("second").unwrap_err(),
        403,
        "forbidden",
    );
    assert!(matches!(e, MaidanError::Forbidden(_)));
    let e = check(
        c.workspaces()
            .import(&bundle, Some(ImportMode::Restore))
            .unwrap_err(),
        409,
        "conflict",
    );
    assert!(matches!(e, MaidanError::Conflict(_)));
}
