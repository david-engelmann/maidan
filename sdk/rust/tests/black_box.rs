//! Black-box tests against the authenticated server from `scripts/sdk-test.sh`. Each
//! test skips (returns) when MAIDAN_URL is unset, matching the repo's Docker-skip
//! convention. These scenarios also exercise the server's REST + WS surface.

use std::sync::mpsc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use maidan::Client;
use serde_json::{json, Value};

fn base() -> Option<String> {
    std::env::var("MAIDAN_URL").ok()
}

static SEED_ID: AtomicU64 = AtomicU64::new(1);

// Create an isolated queue in the token's bootstrap workspace.
fn seed(c: &Client, base: &str) -> (Value, Value, Value, Value) {
    let wid = std::env::var("MAIDAN_WORKSPACE").unwrap();
    let token = std::env::var("MAIDAN_TOKEN").unwrap();
    let me: Value = ureq::get(&format!("{base}/me"))
        .set("authorization", &format!("Bearer {token}"))
        .call()
        .unwrap()
        .into_json()
        .unwrap();
    let ws = json!({ "id": wid });
    let member = json!({ "id": me["member_id"] });
    let name = format!("rust-sdk-{}", SEED_ID.fetch_add(1, Ordering::Relaxed));
    let channel = c.channels().create(&wid, &name, false).unwrap();
    let thread = c
        .threads()
        .create(channel["id"].as_str().unwrap(), "kickoff")
        .unwrap();
    (ws, member, channel, thread)
}

#[test]
fn hero_loop_post_list_context() {
    let Some(base) = base() else {
        eprintln!("skip: MAIDAN_URL unset");
        return;
    };
    let c = Client::new(&base, std::env::var("MAIDAN_TOKEN").unwrap_or_default());
    let (_ws, _member, _ch, thread) = seed(&c, &base);
    let tid = thread["id"].as_str().unwrap();
    c.messages()
        .post(tid, "hello from the rust sdk")
        .unwrap();
    let msgs = c.messages().list(tid, &[]).unwrap();
    assert!(msgs
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["body"] == "hello from the rust sdk"));
    assert!(c.threads().context(tid, &[]).unwrap().is_object());
}

#[test]
fn get_result_unset_is_404() {
    // Exercise the result route and client error path before a result exists.
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, std::env::var("MAIDAN_TOKEN").unwrap_or_default());
    let (_ws, _m, _ch, thread) = seed(&c, &base);
    let err = c
        .threads()
        .get_result(thread["id"].as_str().unwrap())
        .unwrap_err();
    assert_eq!(err.status, 404);
}

#[test]
fn errors_surface_status() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, std::env::var("MAIDAN_TOKEN").unwrap_or_default());
    let err = c
        .threads()
        .get("00000000-0000-0000-0000-000000000000")
        .unwrap_err();
    assert!(err.status >= 400);
}

#[test]
fn claim_returns_the_thread_flattened_not_nested() {
    // The seeded thread is ready, so this claims it. The shape assertions are the
    // point: a nested `thread` key would make every README snippet a silent no-op.
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, std::env::var("MAIDAN_TOKEN").unwrap_or_default());
    let (_ws, member, ch, thread) = seed(&c, &base);
    let claim = c
        .claim_next_thread(ch["id"].as_str().unwrap(), json!({}))
        .unwrap();
    assert!(
        !claim.is_null(),
        "a freshly seeded ready thread should be claimable"
    );
    assert!(
        claim.get("thread").is_none(),
        "thread fields are flattened, not nested"
    );
    assert_eq!(claim["id"], thread["id"]);
    assert_eq!(claim["assignee_id"], member["id"]);
    assert!(
        claim["claim_lease_id"].is_string(),
        "the fencing token renew_claim needs"
    );
    assert!(claim["pin"]["uri"].is_string() && claim["pin"]["content_hash"].is_string());
}

#[test]
fn renew_claim_extends_the_lease_with_the_fencing_token() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, std::env::var("MAIDAN_TOKEN").unwrap_or_default());
    let (_ws, _member, ch, _t) = seed(&c, &base);
    let claim = c
        .claim_next_thread(
            ch["id"].as_str().unwrap(),
            json!({ "lease_secs": 60 }),
        )
        .unwrap();
    let renewed = c
        .renew_claim(
            claim["id"].as_str().unwrap(),
            claim["claim_lease_id"].as_str().unwrap(),
            600,
        )
        .unwrap();
    assert!(
        renewed["assignment_expires_at"].as_str() > claim["assignment_expires_at"].as_str(),
        "lease not extended"
    );
}

#[test]
fn claim_next_returns_null_once_drained() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, std::env::var("MAIDAN_TOKEN").unwrap_or_default());
    let (_ws, _member, ch, _t) = seed(&c, &base);
    let cid = ch["id"].as_str().unwrap();
    let body = json!({});
    c.claim_next_thread(cid, body.clone()).unwrap();
    assert!(c.claim_next_thread(cid, body).unwrap().is_null());
}

#[test]
fn subscribe_delivers_a_message() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, std::env::var("MAIDAN_TOKEN").unwrap_or_default());
    let (ws, _member, _ch, thread) = seed(&c, &base);
    let (tx, rx) = mpsc::channel();
    let tid = thread["id"].as_str().unwrap().to_string();
    let sub = c
        .subscribe(
            json!({ "workspace_id": ws["id"], "kinds": ["message_posted"] }),
            move |e| {
                if e["thread_id"].as_str() == Some(tid.as_str()) {
                    let _ = tx.send(e);
                }
            },
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(200)); // let the subscription attach
    c.messages()
        .post(thread["id"].as_str().unwrap(), "ws ping")
        .unwrap();
    let e = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("did not receive the message_posted event");
    assert_eq!(e["kind"], "message_posted");
    sub.close();
}

#[test]
fn provisioning_seeds_a_member_and_mints_a_scoped_token() {
    // The first thing an integrator does after `maidan init`. Both calls were
    // reachable only through the private transport before.
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, std::env::var("MAIDAN_TOKEN").unwrap_or_default());
    let wid = std::env::var("MAIDAN_WORKSPACE").unwrap();
    let handle = format!("provisioned-{}", SEED_ID.fetch_add(1, Ordering::Relaxed));

    let member = c
        .members()
        .create(&wid, &handle, "agent", None)
        .unwrap();
    assert_eq!(member["handle"], handle);
    assert_eq!(member["kind"], "agent");
    let listed = c.members().list(&wid).unwrap();
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == member["id"]));

    let minted = c
        .tokens()
        .mint(
            &wid,
            member["id"].as_str().unwrap(),
            &["workspace:read"],
            &maidan::MintOptions {
                label: Some("scoped worker"),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        minted["secret"].as_str().is_some_and(|s| !s.is_empty()),
        "the secret is returned once, in the mint response"
    );

    let tokens = c
        .tokens()
        .list(&wid, member["id"].as_str().unwrap())
        .unwrap();
    assert!(
        tokens
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["secret"].is_null()),
        "listing must never return a secret"
    );
}
